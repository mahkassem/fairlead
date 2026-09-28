//! The paths under the migration directories in HEAD's commit, kept in
//! `.git/fairlead/head-paths/<commit>`, so the write hook can tell whether
//! a migration exists at HEAD without starting `git`. HEAD's commit comes
//! from the ref files; a miss costs one `git ls-tree`, which the check and
//! commit stages spend for the current HEAD as they run.

use std::collections::HashSet;
use std::path::Path;

use crate::git;

/// Commits kept, so the cache can't grow with history.
const KEEP: usize = 4;

/// Whether `path`, relative to `root`, is in HEAD's commit tree; `dirs`,
/// relative to `root`, are what the cache lists.
pub fn exists_at_head(root: &Path, dirs: &[String], path: &str) -> bool {
    match head_paths(root, dirs) {
        Some((prefix, paths)) => paths.contains(&format!("{prefix}{path}")),
        None => git::exists_at(root, "HEAD", path),
    }
}

/// Lists HEAD's paths under `dirs` into the cache, unless they are there.
pub fn warm(root: &Path, dirs: &[String]) {
    let _ = head_paths(root, dirs);
}

/// `root`'s place in the worktree, and HEAD's paths under `dirs` from
/// the worktree's top; none when HEAD can't be read from the files.
fn head_paths(root: &Path, dirs: &[String]) -> Option<(String, HashSet<String>)> {
    let redirected = ["GIT_DIR", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY"];
    if redirected.iter().any(|v| std::env::var_os(v).is_some()) {
        return None;
    }
    let (top, git_dir) = git::repo(root)?;
    let prefix = within(root, &top)?;
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => git_dir.join(text.trim_end()),
        Err(_) => git_dir.clone(),
    };
    let commit = head(&git_dir, &common)?;
    let specs: Vec<String> = if dirs.iter().any(String::is_empty) {
        vec![prefix.clone()]
    } else {
        dirs.iter().map(|d| format!("{prefix}{d}")).collect()
    };
    let specs: Vec<String> = specs.into_iter().filter(|s| !s.is_empty()).collect();
    let dir = common.join("fairlead").join("head-paths");
    let paths = match read(&dir.join(&commit), &specs) {
        Some(paths) => paths,
        None => build(root, &dir, &commit, &specs)?,
    };
    Some((prefix, paths))
}

/// `root` relative to the worktree's top, as a prefix for tree paths.
fn within(root: &Path, top: &Path) -> Option<String> {
    let root = std::fs::canonicalize(root).ok()?;
    let mut prefix = String::new();
    for part in root.strip_prefix(top).ok()? {
        prefix += part.to_str()?;
        prefix.push('/');
    }
    Some(prefix)
}

/// The commit HEAD names: its own id when detached, else its branch's,
/// from the loose ref or `packed-refs`.
fn head(git_dir: &Path, common: &Path) -> Option<String> {
    let text = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let id = match text.trim_end().strip_prefix("ref: ") {
        None => text.trim_end().to_string(),
        Some(name) => match std::fs::read_to_string(common.join(name)) {
            Ok(text) => text.trim_end().to_string(),
            Err(_) => std::fs::read_to_string(common.join("packed-refs"))
                .ok()?
                .lines()
                .filter(|l| !l.starts_with(['#', '^']))
                .find_map(|l| l.split_once(' ').filter(|(_, r)| *r == name))?
                .0
                .to_string(),
        },
    };
    let hex = matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit());
    hex.then(|| id.to_ascii_lowercase())
}

/// A cache file is NUL-separated: how many pathspecs listed it, each of
/// them, then the paths. Other pathspecs mean the configuration moved.
fn read(file: &Path, specs: &[String]) -> Option<HashSet<String>> {
    let bytes = std::fs::read(file).ok()?;
    let mut records = bytes.split(|&b| b == 0).map(String::from_utf8_lossy);
    let count: usize = records.next()?.parse().ok()?;
    let listed: Vec<_> = records.by_ref().take(count).collect();
    if listed.len() != specs.len() || listed.iter().zip(specs).any(|(a, b)| a != b) {
        return None;
    }
    Some(
        records
            .filter(|r| !r.is_empty())
            .map(|r| r.into_owned())
            .collect(),
    )
}

/// Lists the commit's paths with one `git ls-tree`, and writes them
/// through a temporary file, so a reader never sees half a list.
fn build(root: &Path, dir: &Path, commit: &str, specs: &[String]) -> Option<HashSet<String>> {
    let mut args = vec!["ls-tree", "--full-tree", "-r", "--name-only", "-z", commit];
    args.extend(specs.iter().map(String::as_str));
    let listed = git::git(root, &args).ok()?;
    let mut bytes = format!("{}\0", specs.len()).into_bytes();
    for spec in specs {
        bytes.extend_from_slice(spec.as_bytes());
        bytes.push(0);
    }
    bytes.extend_from_slice(&listed);
    // The cache only saves time: failing to write it never fails the answer.
    if std::fs::create_dir_all(dir).is_ok() {
        let temp = dir.join(format!("{commit}.{}.tmp", std::process::id()));
        if std::fs::write(&temp, &bytes).is_ok() && std::fs::rename(&temp, dir.join(commit)).is_ok()
        {
            prune(dir, commit);
        }
        let _ = std::fs::remove_file(&temp);
    }
    let paths = listed.split(|&b| b == 0).filter(|p| !p.is_empty());
    Some(
        paths
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect(),
    )
}

/// Removes all but the newest few files, never the one just written.
fn prune(dir: &Path, commit: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut others: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name() != commit)
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    others.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in others.into_iter().skip(KEEP - 1) {
        let _ = std::fs::remove_file(path);
    }
}
