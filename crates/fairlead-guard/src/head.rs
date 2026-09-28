//! Whether a path is in HEAD's tree, read from the files under `.git`,
//! since the write hook's budget can't spare a `git` process. The index
//! lists what is staged, not what HEAD holds, so it answers only while its
//! cached root tree is HEAD's tree: after a commit or a checkout, until
//! something is staged. A failed commit also leaves the cached tree
//! whole, with the staged files in it, which is why the tree is compared
//! with HEAD's rather than trusted. Any other state is left to `git`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::inflate::inflate;

/// Whether `path`, relative to `root`, is in HEAD's tree; none when the
/// files can't say for sure, and `git` has to.
pub fn in_head(root: &Path, path: &str) -> Option<bool> {
    let redirected = [
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
    ];
    if redirected.iter().any(|v| std::env::var_os(v).is_some()) {
        return None;
    }
    let (top, git_dir) = crate::git::repo(root)?;
    let full = within(root, &top)? + path;
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => git_dir.join(text.trim_end()),
        Err(_) => git_dir.clone(),
    };
    let head = head(&git_dir, &common)?;
    let tree = commit_tree(&common.join("objects"), &head)?;
    let index = std::fs::read(git_dir.join("index")).ok()?;
    let (cached, found) = lookup(&index, head.len(), full.as_bytes())?;
    (cached == tree.as_slice()).then_some(found)
}

/// `root` relative to the worktree's top, as a prefix for index paths.
fn within(root: &Path, top: &Path) -> Option<String> {
    let root = std::fs::canonicalize(root).ok()?;
    let mut prefix = String::new();
    for part in root.strip_prefix(top).ok()? {
        prefix += part.to_str()?;
        prefix.push('/');
    }
    Some(prefix)
}

fn hex(text: &str) -> Option<Vec<u8>> {
    if text.len() != 40 && text.len() != 64 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The commit HEAD names, through a branch's loose or packed ref.
fn head(git_dir: &Path, common: &Path) -> Option<Vec<u8>> {
    let mut text = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    for _ in 0..5 {
        let Some(name) = text.trim_end().strip_prefix("ref: ") else {
            return hex(text.trim_end());
        };
        let name = name.to_string();
        text = match std::fs::read_to_string(common.join(&name)) {
            Ok(text) => text,
            Err(_) => std::fs::read_to_string(common.join("packed-refs"))
                .ok()?
                .lines()
                .filter(|l| !l.starts_with(['#', '^']))
                .find_map(|l| l.split_once(' ').filter(|(_, r)| *r == name))?
                .0
                .to_string(),
        };
    }
    None
}

/// The root tree of a commit stored loose or whole in a pack; none for a
/// deltified one, which would mean resolving its base.
fn commit_tree(objects: &Path, commit: &[u8]) -> Option<Vec<u8>> {
    let hexed: String = commit.iter().map(|b| format!("{b:02x}")).collect();
    let want = 2 * hexed.len() + 64;
    let text = match read_start(&objects.join(&hexed[..2]).join(&hexed[2..]), 0) {
        Some(bytes) => {
            let body = inflate(&bytes, want)?;
            let nul = body.iter().position(|&b| b == 0)?;
            body.starts_with(b"commit ")
                .then(|| body[nul + 1..].to_vec())?
        }
        None => packed_commit(objects, commit, want)?,
    };
    let rest = text.strip_prefix(b"tree ")?;
    hex(std::str::from_utf8(rest.get(..hexed.len())?).ok()?)
}

/// Up to 8 KiB of a file from `offset`, plenty for a commit's first line.
fn read_start(path: &Path, offset: u64) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    file.take(8192).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn packed_commit(objects: &Path, commit: &[u8], want: usize) -> Option<Vec<u8>> {
    let packs = std::fs::read_dir(objects.join("pack")).ok()?;
    let (pack, offset) = packs.filter_map(Result::ok).find_map(|e| {
        let idx: PathBuf = e.path();
        let offset = idx
            .extension()
            .is_some_and(|x| x == "idx")
            .then(|| pack_offset(&idx, commit))??;
        Some((idx.with_extension("pack"), offset))
    })?;
    let bytes = read_start(&pack, offset)?;
    // The entry header: its type in bits 4 to 6, then a varint size.
    if (bytes.first()? >> 4) & 7 != 1 {
        return None;
    }
    let start = bytes.iter().position(|b| b & 0x80 == 0)? + 1;
    inflate(bytes.get(start..)?, want)
}

/// Where a version 2 pack index puts an object in its pack.
fn pack_offset(idx: &Path, oid: &[u8]) -> Option<u64> {
    let mut file = File::open(idx).ok()?;
    let mut at = |pos: u64, buf: &mut [u8]| -> Option<()> {
        file.seek(SeekFrom::Start(pos)).ok()?;
        file.read_exact(buf).ok()
    };
    let mut head = [0u8; 8 + 256 * 4];
    at(0, &mut head)?;
    if head[..8] != [0xff, b't', b'O', b'c', 0, 0, 0, 2] {
        return None;
    }
    let fan = |i: usize| {
        u64::from(u32::from_be_bytes(
            [0, 1, 2, 3].map(|k| head[8 + i * 4 + k]),
        ))
    };
    let first = usize::from(*oid.first()?);
    let (lo, hi, n) = (first.checked_sub(1).map_or(0, fan), fan(first), fan(255));
    let len = oid.len() as u64;
    let table = 8 + 256 * 4;
    let mut ids = vec![0u8; usize::try_from(hi.checked_sub(lo)? * len).ok()?];
    at(table + lo * len, &mut ids)?;
    let i = lo + ids.chunks(oid.len()).position(|id| id == oid)? as u64;
    let mut word = [0u8; 4];
    at(table + n * len + n * 4 + i * 4, &mut word)?;
    let offset = u32::from_be_bytes(word);
    if offset & 0x8000_0000 == 0 {
        return Some(u64::from(offset));
    }
    let mut wide = [0u8; 8];
    at(
        table + n * (len + 8) + u64::from(offset & 0x7fff_ffff) * 8,
        &mut wide,
    )?;
    Some(u64::from_be_bytes(wide))
}

fn be(data: &[u8], at: usize, width: usize) -> Option<usize> {
    let bytes = data.get(at..at + width)?;
    Some(bytes.iter().fold(0, |n, &b| (n << 8) | usize::from(b)))
}

/// Git's offset varint, which version 4 uses for the bytes a name drops
/// from the one before it.
fn varint(data: &[u8]) -> Option<(usize, usize)> {
    let mut value = usize::from(*data.first()? & 0x7f);
    let mut used = 1;
    while data[used - 1] & 0x80 != 0 {
        value = ((value + 1) << 7) | usize::from(*data.get(used)? & 0x7f);
        used += 1;
    }
    Some((value, used))
}

/// An index's cached root tree, and whether `path` is in it; none for an
/// index this can't read in full: another version, an extension git would
/// refuse to skip (a split or sparse index), a conflict or an intent-to-add
/// entry on the path, or a root tree invalidated by a staged change.
fn lookup<'a>(data: &'a [u8], hash: usize, path: &[u8]) -> Option<(&'a [u8], bool)> {
    let version = be(data, 4, 4)?;
    if data.get(..4)? != b"DIRC" || !(2..=4).contains(&version) {
        return None;
    }
    let (mut pos, mut name, mut found) = (12, Vec::new(), false);
    for _ in 0..be(data, 8, 4)? {
        let flags = be(data, pos + 40 + hash, 2)?;
        let mut at = pos + 40 + hash + 2;
        let mut extended = 0;
        if flags & 0x4000 != 0 {
            extended = be(data, at, 2).filter(|_| version >= 3)?;
            at += 2;
        }
        if version == 4 {
            let (drop, used) = varint(data.get(at..)?)?;
            name.truncate(name.len().checked_sub(drop)?);
            at += used;
        } else {
            name.clear();
        }
        let len = data.get(at..)?.iter().position(|&b| b == 0)?;
        name.extend_from_slice(&data[at..at + len]);
        pos = if version == 4 {
            at + len + 1
        } else {
            pos + ((at - pos + len + 8) & !7)
        };
        if name == path {
            if flags & 0x3000 != 0 || extended & 0x2000 != 0 {
                return None;
            }
            found = true;
        }
    }
    let end = data.len().checked_sub(hash)?;
    let mut tree = None;
    while pos + 8 <= end {
        let (sig, size) = (data.get(pos..pos + 4)?, be(data, pos + 4, 4)?);
        let body = data.get(pos + 8..pos + 8 + size)?;
        match sig {
            b"TREE" => tree = root_tree(body, hash),
            _ if !sig[0].is_ascii_uppercase() => return None,
            _ => {}
        }
        pos += 8 + size;
    }
    Some((tree?, found))
}

/// The first cache-tree entry is the root: an empty name, its entry count
/// (-1 once something under it is staged), its subtree count, its tree.
fn root_tree(body: &[u8], hash: usize) -> Option<&[u8]> {
    let line = body.strip_prefix(b"\0")?;
    let nl = line.iter().position(|&b| b == b'\n')?;
    let (count, _) = std::str::from_utf8(&line[..nl]).ok()?.split_once(' ')?;
    if count.parse::<i64>().ok()? < 0 {
        return None;
    }
    line.get(nl + 1..nl + 1 + hash)
}
