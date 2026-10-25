//! `plan --head REV`: a commit planned without checking it out, from a
//! temporary detached worktree of it that is removed afterwards. The graph
//! and the config are the commit's own; the parse cache is the caller's
//! checkout's, since it's keyed by content.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use fairlead_tests::git;

/// Temporary worktrees are named so a later run can tell its own leftovers.
const PREFIX: &str = "fairlead-head-";
/// Where a process can't be asked whether it's alive, a leftover this old is
/// taken to be abandoned.
const ABANDONED: Duration = Duration::from_secs(3600);

fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A temporary detached worktree at one commit, removed when dropped.
pub struct Checkout {
    repo: PathBuf,
    pub dir: PathBuf,
}

impl Drop for Checkout {
    fn drop(&mut self) {
        remove(&self.repo, &self.dir);
    }
}

fn remove(repo: &Path, dir: &Path) {
    let _ = run(
        repo,
        &["worktree", "remove", "--force", &dir.to_string_lossy()],
    );
    if dir.exists() {
        let _ = std::fs::remove_dir_all(dir);
    }
    let _ = run(repo, &["worktree", "prune"]);
}

/// The top of the repository holding `cwd`.
pub fn top(cwd: &Path) -> Result<PathBuf, String> {
    let top = run(cwd, &["rev-parse", "--show-toplevel"])
        .map_err(|_| "--head needs a git repository".to_string())?;
    let top = PathBuf::from(top);
    Ok(std::fs::canonicalize(&top).unwrap_or(top))
}

/// `rev` as a commit id, read in `repo`.
pub fn commit(repo: &Path, rev: &str) -> Result<String, String> {
    run(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .ok()
    .filter(|c| !c.is_empty())
    .ok_or_else(|| format!("{rev} isn't a commit in this repository"))
}

/// A detached worktree of `rev` under the system temp directory. Hooks are
/// off, so a post-checkout hook can't install anything there, and Git LFS
/// leaves pointer files instead of downloading.
pub fn check_out(repo: &Path, rev: &str) -> Result<Checkout, String> {
    sweep(repo);
    let commit = commit(repo, rev)?;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("{PREFIX}{}-{nanos}", std::process::id()));
    let no_hooks = format!("core.hooksPath={}.no-hooks", dir.display());
    let added = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", &no_hooks, "worktree", "add", "--detach", "--quiet"])
        .arg(&dir)
        .arg(&commit)
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .output()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    if !added.status.success() {
        remove(repo, &dir);
        return Err(format!(
            "could not check out {rev}: {}",
            String::from_utf8_lossy(&added.stderr).trim()
        ));
    }
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    Ok(Checkout {
        repo: repo.to_path_buf(),
        dir,
    })
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

/// Removes worktrees an interrupted run left behind: registered, named by
/// this module, and owned by a process that's gone.
fn sweep(repo: &Path) {
    let Ok(listing) = run(repo, &["worktree", "list", "--porcelain"]) else {
        return;
    };
    let leftovers: Vec<PathBuf> = listing
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .map(PathBuf::from)
        .filter(|p| abandoned(p))
        .collect();
    for dir in &leftovers {
        remove(repo, dir);
    }
    let _ = run(repo, &["worktree", "prune"]);
}

fn abandoned(dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Some(pid) = name
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.split('-').next())
        .and_then(|p| p.parse::<i32>().ok())
        .filter(|p| *p > 0)
    else {
        return false;
    };
    if !dir.exists() {
        return true;
    }
    if cfg!(unix) {
        let alive = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .is_ok_and(|o| o.status.success());
        return !alive;
    }
    dir.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > ABANDONED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_named_worktree_of_a_gone_process_is_abandoned() {
        let gone = std::env::temp_dir().join(format!("{PREFIX}999999999-1"));
        assert!(abandoned(&gone), "a missing directory is a leftover");
        let mine = std::env::temp_dir().join(format!("{PREFIX}{}-1", std::process::id()));
        std::fs::create_dir_all(&mine).unwrap();
        assert!(!abandoned(&mine), "this process is alive");
        let _ = std::fs::remove_dir_all(&mine);
        assert!(!abandoned(Path::new("/tmp/someone-elses-worktree")));
        assert!(!abandoned(Path::new("/tmp/fairlead-head-notapid-1")));
        assert!(
            !abandoned(Path::new("/tmp/fairlead-head-0-1")),
            "kill -0 0 is everyone"
        );
    }
}
