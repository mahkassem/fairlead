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

/// Files the next commit adds, changes or renames. Before the first commit
/// everything staged is added.
pub fn staged(root: &Path) -> Result<Vec<Staged>, String> {
    let has_head = git(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok();
    let out = git(
        root,
        &[
            "diff",
            "--cached",
            "--name-status",
            "-z",
            "-M",
            "--diff-filter=ACMR",
            "--relative",
        ],
    )?;
    let fields = nul_split(&out);
    let mut staged = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let status = fields[i].as_bytes().first().copied().unwrap_or(b'?');
        if status == b'R' || status == b'C' {
            let (old, new) = (fields.get(i + 1), fields.get(i + 2));
            if let (Some(old), Some(new)) = (old, new) {
                let before = (status == b'R' && has_head).then(|| old.clone());
                staged.push(Staged {
                    before,
                    path: new.clone(),
                });
            }
            i += 3;
        } else {
            if let Some(path) = fields.get(i + 1) {
                let before = (status == b'M' && has_head).then(|| path.clone());
                staged.push(Staged {
                    before,
                    path: path.clone(),
                });
            }
            i += 2;
        }
    }
    Ok(staged)
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
