//! Temporary detached worktrees of one commit, removed when dropped: the
//! whole tree, or for a tool that takes its files, only those files and the
//! files beside and above them, where its config lives.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

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
    add(repo, rev, false)
}

/// A worktree of `rev` holding only `files`, each of the files directly in
/// their folders and every folder above them, and the repository's ignored
/// top-level folders, such as installed dependencies, linked from `repo`.
pub fn check_out_files(repo: &Path, rev: &str, files: &[String]) -> Result<Checkout, String> {
    let checkout = add(repo, rev, true)?;
    let commit = commit(repo, rev)?;
    let mut paths: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = vec![String::new()];
    for f in files {
        let mut dir = f.as_str();
        while let Some((parent, _)) = dir.rsplit_once('/') {
            if !dirs.iter().any(|d| d == parent) {
                dirs.push(parent.to_string());
            }
            dir = parent;
        }
    }
    for dir in &dirs {
        let spec = if dir.is_empty() {
            ".".to_string()
        } else {
            format!("{dir}/")
        };
        let listing = run(repo, &["ls-tree", "-z", &commit, "--", &spec]).unwrap_or_default();
        for entry in listing.split('\0').filter(|e| !e.is_empty()) {
            if let Some((meta, path)) = entry.split_once('\t') {
                if meta.split(' ').nth(1) == Some("blob") && !paths.contains(&path.to_string()) {
                    paths.push(path.to_string());
                }
            }
        }
    }
    let existing = run(repo, &["ls-tree", "-r", "-z", "--name-only", &commit]).unwrap_or_default();
    let existing: std::collections::HashSet<&str> =
        existing.split('\0').filter(|p| !p.is_empty()).collect();
    for f in files {
        if existing.contains(f.as_str()) && !paths.contains(f) {
            paths.push(f.clone());
        }
    }
    for chunk in paths.chunks(200) {
        let mut args = vec!["checkout", commit.as_str(), "--"];
        args.extend(chunk.iter().map(String::as_str));
        run(&checkout.dir, &args)?;
    }
    link_ignored(repo, &checkout.dir);
    Ok(checkout)
}

/// Links each ignored top-level folder of `repo`, such as `node_modules`,
/// into the checkout, so a tool finds what the project installed.
fn link_ignored(repo: &Path, dir: &Path) {
    let Ok(listing) = run(
        repo,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
        ],
    ) else {
        return;
    };
    for name in listing.split('\0').filter_map(|p| p.strip_suffix('/')) {
        if name.is_empty() || name.contains('/') || dir.join(name).exists() {
            continue;
        }
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(repo.join(name), dir.join(name));
    }
}

fn add(repo: &Path, rev: &str, bare: bool) -> Result<Checkout, String> {
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
        .args(bare.then_some("--no-checkout"))
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

    #[test]
    fn a_files_checkout_holds_the_files_and_those_beside_and_above_them_only() {
        let repo = std::env::temp_dir().join(format!("fairlead-sparse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        for (path, text) in [
            ("lint.toml", "x"),
            ("a/x.txt", "x"),
            ("a/b/y.txt", "y"),
            ("a/b/deeper/w.txt", "w"),
            ("c/z.txt", "z"),
        ] {
            let full = repo.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        let git = |args: &[&str]| {
            Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "x"]);
        let checkout = check_out_files(&repo, "HEAD", &["a/b/y.txt".into()]).unwrap();
        let has = |p: &str| checkout.dir.join(p).is_file();
        assert!(has("lint.toml") && has("a/x.txt") && has("a/b/y.txt"));
        assert!(!has("c/z.txt") && !has("a/b/deeper/w.txt"));
        let dir = checkout.dir.clone();
        drop(checkout);
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(&repo);
    }
}
