//! The files Fairlead reads: everything git doesn't ignore, never
//! `node_modules` or `.git`. Paths are repo-relative with `/` separators on
//! every platform, so they match globs and print the same everywhere.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const SOURCE_EXTENSIONS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
const SKIPPED_DIRS: [&str; 2] = [".git", "node_modules"];

#[derive(Debug, Clone)]
pub struct Tree {
    pub root: PathBuf,
    pub files: Vec<String>,
    set: HashSet<String>,
}

impl Tree {
    pub fn scan(root: &Path) -> Tree {
        let walker = ignore::WalkBuilder::new(root)
            .hidden(false)
            .require_git(false)
            .filter_entry(|e| !SKIPPED_DIRS.iter().any(|d| e.file_name() == *d))
            .build();
        let mut files: Vec<String> = walker
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .filter_map(|e| relative(root, e.path()))
            .collect();
        files.sort();
        Tree::from_files(root, files)
    }

    pub fn from_files(root: &Path, files: Vec<String>) -> Tree {
        let set = files.iter().cloned().collect();
        Tree {
            root: root.to_path_buf(),
            files,
            set,
        }
    }

    pub fn contains(&self, rel: &str) -> bool {
        self.set.contains(rel)
    }

    pub fn is_source(rel: &str) -> bool {
        rel.rsplit_once('.')
            .is_some_and(|(stem, ext)| SOURCE_EXTENSIONS.contains(&ext) && !stem.ends_with(".d"))
    }

    pub fn sources(&self) -> impl Iterator<Item = &String> {
        self.files.iter().filter(|f| Tree::is_source(f))
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn rel(&self, abs: &Path) -> Option<String> {
        relative(&self.root, abs)
    }
}

pub fn relative(root: &Path, abs: &Path) -> Option<String> {
    let (root, abs) = (plain(root), plain(abs));
    let rel = abs.strip_prefix(&root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// `path` without Windows' `\\?\` prefix, which `std::fs::canonicalize`
/// adds and the resolver strips, so the two compare equal.
pub fn plain(path: &Path) -> PathBuf {
    if cfg!(windows) {
        if let Some(s) = path.to_str().and_then(strip_verbatim) {
            return PathBuf::from(s);
        }
    }
    path.to_path_buf()
}

fn strip_verbatim(path: &str) -> Option<String> {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{rest}"));
    }
    let rest = path.strip_prefix(r"\\?\")?;
    let drive = rest.as_bytes();
    (drive.len() >= 2 && drive[0].is_ascii_alphabetic() && drive[1] == b':')
        .then(|| rest.to_string())
}

/// Joins a `/`-separated path onto a directory and resolves `.` and `..`,
/// without touching the filesystem. `None` if it climbs above the root.
pub fn normalize(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if rel.starts_with('/') {
        Vec::new()
    } else {
        dir.split('/').filter(|p| !p.is_empty()).collect()
    };
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub fn parent(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_resolves_dots_and_refuses_to_climb_out() {
        assert_eq!(normalize("a/b", "../c/d.js").as_deref(), Some("a/c/d.js"));
        assert_eq!(normalize("a", "./x.json").as_deref(), Some("a/x.json"));
        assert_eq!(normalize("a", "../../x"), None);
        assert_eq!(normalize("a/b", "/top.ts").as_deref(), Some("top.ts"));
    }

    #[test]
    fn verbatim_prefixes_drop_only_for_disks_and_shares() {
        assert_eq!(strip_verbatim(r"\\?\D:\a\b").as_deref(), Some(r"D:\a\b"));
        assert_eq!(
            strip_verbatim(r"\\?\UNC\srv\share\x").as_deref(),
            Some(r"\\srv\share\x")
        );
        assert_eq!(strip_verbatim(r"\\?\Volume{1}\x"), None);
        assert_eq!(strip_verbatim(r"D:\a"), None);
    }

    #[test]
    fn declaration_files_are_not_sources() {
        assert!(Tree::is_source("src/a.ts"));
        assert!(Tree::is_source("src/a.test.tsx"));
        assert!(!Tree::is_source("src/a.d.ts"));
        assert!(!Tree::is_source("src/a.json"));
    }
}
