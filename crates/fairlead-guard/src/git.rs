//! The files each stage reads. The check stage reads what git tracks, as it
//! is on disk; the commit stage reads what is staged, from the index, since
//! the working tree can hold changes the commit won't.

use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git {}: {}", args.join(" "), err.trim()));
    }
    Ok(out.stdout)
}

fn nul_split(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Every tracked path, relative to `root`.
pub fn tracked(root: &Path) -> Result<Vec<String>, String> {
    Ok(nul_split(&git(root, &["ls-files", "-z"])?))
}

/// A staged file: where it was at HEAD, if anywhere, and where it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub before: Option<String>,
    pub path: String,
}

/// One path in a diff: its status letter (`A`, `C`, `D`, `M`, `R`, `T`),
/// the path it came from for a rename or copy, and where it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub status: char,
    pub before: Option<String>,
    pub path: String,
}

/// What a diff compares the tree with.
#[derive(Debug, Clone, Copy)]
pub enum Against<'a> {
    /// HEAD against the index: what the next commit changes.
    Index,
    /// A revision against the working tree.
    Rev(&'a str),
}

pub fn changes(root: &Path, against: Against<'_>) -> Result<Vec<Change>, String> {
    let mut args = vec!["diff", "--name-status", "-z", "-M", "--relative"];
    match against {
        Against::Index => args.push("--cached"),
        Against::Rev(rev) => args.push(rev),
    }
    let fields = nul_split(&git(root, &args)?);
    let mut out = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let status = fields[i].chars().next().unwrap_or('?');
        let pair = matches!(status, 'R' | 'C');
        let (before, path) = if pair {
            (fields.get(i + 1).cloned(), fields.get(i + 2))
        } else {
            (None, fields.get(i + 1))
        };
        if let Some(path) = path {
            out.push(Change {
                status,
                before,
                path: path.clone(),
            });
        }
        i += if pair { 3 } else { 2 };
    }
    Ok(out)
}

/// Files the next commit adds, changes or renames. Before the first commit
/// everything staged is added.
pub fn staged(root: &Path) -> Result<Vec<Staged>, String> {
    let has_head = git(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok();
    Ok(changes(root, Against::Index)?
        .into_iter()
        .filter(|c| matches!(c.status, 'A' | 'C' | 'M' | 'R'))
        .map(|c| {
            let before = match c.status {
                'R' if has_head => c.before,
                'M' if has_head => Some(c.path.clone()),
                _ => None,
            };
            Staged {
                before,
                path: c.path,
            }
        })
        .collect())
}

/// The commit HEAD and `rev` share.
pub fn merge_base(root: &Path, rev: &str) -> Result<String, String> {
    let out = git(root, &["merge-base", "HEAD", rev])?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// Every path in a revision's tree, relative to `root`; none before the first commit.
pub fn paths_at(root: &Path, rev: &str) -> std::collections::HashSet<String> {
    git(root, &["ls-tree", "-r", "-z", "--name-only", rev])
        .map(|out| nul_split(&out).into_iter().collect())
        .unwrap_or_default()
}

/// A file's text at HEAD; `./` makes the path relative to `root`, as the listings are.
pub fn at_head(root: &Path, path: &str) -> Option<String> {
    blob(root, &format!("HEAD:./{path}"))
}

/// A file's text in the index.
pub fn in_index(root: &Path, path: &str) -> Option<String> {
    blob(root, &format!(":./{path}"))
}

/// A blob's text, or none when it's missing or isn't UTF-8.
fn blob(root: &Path, spec: &str) -> Option<String> {
    String::from_utf8(git(root, &["cat-file", "blob", spec]).ok()?).ok()
}
