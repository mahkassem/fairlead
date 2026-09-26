//! The parse cache: each file's extraction result, keyed by its git blob id
//! and extension, kept in the repository's git directory so it's never
//! committed. Anything unreadable or from another version is ignored and
//! rebuilt; the cache only ever saves time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::extract::Extracted;

/// Bumped whenever extraction changes what it returns for the same bytes.
const EXTRACTOR: u32 = 1;
const FILE: &str = "parse-cache.json";

#[derive(Serialize, Deserialize)]
struct Stored {
    version: String,
    entries: HashMap<String, Extracted>,
}

#[derive(Debug, Default)]
pub struct ParseCache {
    dir: Option<PathBuf>,
    entries: HashMap<String, Extracted>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub enabled: bool,
    pub hits: usize,
    pub misses: usize,
}

fn version() -> String {
    format!("{}+{EXTRACTOR}", env!("CARGO_PKG_VERSION"))
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
        let Some(dir) = dir_for(root) else {
            return ParseCache::disabled();
        };
        let entries = std::fs::read(dir.join(FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
            .filter(|stored| stored.version == version())
            .map(|stored| stored.entries)
            .unwrap_or_default();
        ParseCache {
            dir: Some(dir),
            entries,
        }
    }

    pub fn enabled(&self) -> bool {
        self.dir.is_some()
    }

    pub fn get(&self, key: &str) -> Option<&Extracted> {
        self.entries.get(key)
    }

    /// Writes `entries`, the ones this run used, replacing what was there so
    /// files no longer in the tree drop out. A failed write is ignored.
    pub fn save(&self, entries: HashMap<String, Extracted>) {
        let Some(dir) = &self.dir else {
            return;
        };
        let stored = Stored {
            version: version(),
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
    fn the_same_bytes_key_differently_by_extension() {
        assert_ne!(key("a.ts", b"x"), key("a.js", b"x"));
        assert_eq!(key("a.ts", b"x"), key("b/c.ts", b"x"));
    }
}
