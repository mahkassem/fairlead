//! The git operations replay needs, on a clone of the benchmark repository:
//! whether a commit is present, merge bases, and one reused worktree that
//! each failing commit is checked out into, so the planner reads it like
//! any working tree and the parse cache carries over between commits.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn has_commit(clone: &Path, sha: &str) -> bool {
    git(clone, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_ok()
}

pub fn merge_base(clone: &Path, a: &str, b: &str) -> Option<String> {
    git(clone, &["merge-base", "--end-of-options", a, b])
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn tree_of(clone: &Path, sha: &str) -> Option<String> {
    git(clone, &["rev-parse", &format!("{sha}^{{tree}}")]).ok()
}

/// A detached worktree of `clone` at `path`, created on first use.
pub struct Worktree {
    pub path: PathBuf,
}

impl Worktree {
    pub fn open(clone: &Path, path: &Path, at: &str) -> Result<Worktree, String> {
        if !path.join(".git").exists() {
            let _ = git(clone, &["worktree", "prune"]);
            let target = path.to_string_lossy().into_owned();
            git(
                clone,
                &["worktree", "add", "--quiet", "--detach", &target, at],
            )?;
        }
        Ok(Worktree {
            path: path.to_path_buf(),
        })
    }

    pub fn checkout(&self, sha: &str) -> Result<(), String> {
        git(
            &self.path,
            &["checkout", "--quiet", "--detach", "--force", sha],
        )?;
        git(&self.path, &["clean", "-q", "-f", "-d"]).map(|_| ())
    }
}
