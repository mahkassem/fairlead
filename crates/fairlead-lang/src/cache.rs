//! The parse cache: each file's extraction result, keyed by its git blob id
//! and extension, kept in the repository's git directory so it's never
//! committed. Anything unreadable or from another version is ignored and
//! rebuilt; the cache only ever saves time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::extract::{self, Extracted};

/// Bumped when extraction code changes what it returns for the same bytes;
/// query, limit and grammar changes are covered by `extract::fingerprint`.
const EXTRACTOR: u32 = 6;
const FILE: &str = "parse-cache.json";
/// A temp file this old was left by a process that died mid-write.
const STALE_TMP: Duration = Duration::from_secs(600);

#[derive(Serialize, Deserialize)]
struct Stored {
    version: String,
    entries: HashMap<String, Extracted>,
}

#[derive(Debug, Default)]
pub struct ParseCache {
    dir: Option<PathBuf>,
    version: String,
    entries: HashMap<String, Extracted>,
    /// Borrowed from another checkout: a save adds this run's entries and
    /// prunes nothing, so a scan of an old commit leaves the owner's intact.
    shared: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub enabled: bool,
    pub hits: usize,
    pub misses: usize,
}

fn version() -> String {
    format!(
        "{}+{EXTRACTOR}+{}",
        env!("CARGO_PKG_VERSION"),
        extract::fingerprint()
    )
}

/// `git hash-object` for these bytes.
pub fn blob_id(bytes: &[u8]) -> String {
    let mut hasher = sha1_smol::Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hasher.digest().to_string()
}

/// The cache's key for a file: blob id and extension, since the same bytes
/// parse differently as TypeScript and JavaScript.
pub fn key(rel: &str, bytes: &[u8]) -> String {
    let ext = rel.rsplit_once('.').map_or("", |(_, e)| e);
    format!("{}.{ext}", blob_id(bytes))
}

/// `<git dir>/fairlead`, following a worktree's `.git` file to its git dir.
pub fn dir_for(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git.join("fairlead"));
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let git_dir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    Some(root.join(git_dir).join("fairlead"))
}

impl ParseCache {
    pub fn disabled() -> ParseCache {
        ParseCache::default()
    }

    pub fn open(root: &Path) -> ParseCache {
        match dir_for(root) {
            Some(dir) => ParseCache::open_dir(dir, false),
            None => ParseCache::disabled(),
        }
    }

    /// The cache of the checkout at `owner`, for a scan of another checkout
    /// of the same repository: keyed by content, its entries hold for both.
    pub fn borrow(owner: &Path) -> ParseCache {
        match dir_for(owner) {
            Some(dir) => ParseCache::open_dir(dir, true),
            None => ParseCache::disabled(),
        }
    }

    fn open_dir(dir: PathBuf, shared: bool) -> ParseCache {
        let version = version();
        let entries = std::fs::read(dir.join(FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
            .filter(|stored| stored.version == version)
            .map(|stored| stored.entries)
            .unwrap_or_default();
        ParseCache {
            dir: Some(dir),
            version,
            entries,
            shared,
        }
    }

    pub fn enabled(&self) -> bool {
        self.dir.is_some()
    }

    pub fn get(&self, key: &str) -> Option<&Extracted> {
        self.entries.get(key)
    }

    /// Writes `entries`, the ones this run used, replacing what was there so
    /// files no longer in the tree drop out, or added to it when the cache
    /// is borrowed. Nothing is written when that changes nothing, and a
    /// failed write is ignored.
    pub fn save(&self, mut entries: HashMap<String, Extracted>) {
        let Some(dir) = &self.dir else {
            return;
        };
        let known = entries.keys().all(|k| self.entries.contains_key(k));
        if known && (self.shared || entries.len() == self.entries.len()) {
            return;
        }
        if self.shared {
            for (k, v) in &self.entries {
                entries.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        remove_stale_temps(dir);
        let stored = Stored {
            version: self.version.clone(),
            entries,
        };
        let Ok(bytes) = serde_json::to_vec(&stored) else {
            return;
        };
        let tmp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
        let written = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&tmp, bytes))
            .and_then(|()| std::fs::rename(&tmp, dir.join(FILE)));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn remove_stale_temps(dir: &Path) {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in listing.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_TMP);
        if name.starts_with(FILE) && name.ends_with(".tmp") && stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_ids_match_git() {
        // `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(
            blob_id(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        assert_eq!(blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn a_borrowed_cache_adds_entries_and_an_owned_one_prunes_them() {
        let root = std::env::temp_dir().join(format!("fairlead-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let one = |k: &str| HashMap::from([(k.to_string(), Extracted::default())]);
        ParseCache::open(&root).save(one("a"));
        ParseCache::borrow(&root).save(one("b"));
        let both = ParseCache::open(&root);
        assert!(both.get("a").is_some() && both.get("b").is_some());
        both.save(one("b"));
        let pruned = ParseCache::open(&root);
        assert!(pruned.get("a").is_none() && pruned.get("b").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_same_bytes_key_differently_by_extension() {
        assert_ne!(key("a.ts", b"x"), key("a.js", b"x"));
        assert_eq!(key("a.ts", b"x"), key("b/c.ts", b"x"));
    }
}
