//! `plan --head REV`: a commit planned without checking it out, from a
//! temporary detached worktree of it that is removed afterwards. The graph
//! and the config are the commit's own; the parse cache is the caller's
//! checkout's, since it's keyed by content.

use std::path::{Path, PathBuf};

use fairlead_tests::git;

pub use fairlead_guard::worktree::{check_out, commit, Checkout};

/// The top of the repository holding `cwd`.
pub fn top(cwd: &Path) -> Result<PathBuf, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .ok_or_else(|| "--head needs a git repository".to_string())?;
    let top = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    Ok(std::fs::canonicalize(&top).unwrap_or(top))
}

/// The commit `--head` is measured from: `--base` read in the caller's
/// repository, else the head's merge base with the default branch, or its
/// first parent when the head is already on that branch.
pub fn base(repo: &Path, checkout: &Path, given: Option<&str>) -> Result<String, String> {
    if let Some(given) = given {
        return commit(repo, given);
    }
    let default = git::default_base(checkout)?;
    let merge_base = git::merge_base(checkout, &default)?;
    if Some(&merge_base) != git::head(checkout).as_ref() {
        return Ok(merge_base);
    }
    commit(checkout, "HEAD^")
        .map_err(|_| "the head has no parent to compare with; pass --base".to_string())
}
