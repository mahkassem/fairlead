//! The git operations replay needs, on a clone of the benchmark repository:
//! whether a commit is present, merge bases, and one reused worktree that
//! each failing commit is checked out into, so the planner reads it like
//! any working tree and the parse cache carries over between commits.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// `git patch-id --stable` of `base..head`: equal for the same change on
/// another base. An empty diff gives an empty id.
pub fn patch_id(clone: &Path, base: &str, head: &str) -> Option<String> {
    let diff = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["diff", "--end-of-options", base, head])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    if diff.stdout.is_empty() {
        return Some(String::new());
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["patch-id", "--stable"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let writer = std::thread::spawn(move || stdin.write_all(&diff.stdout));
    let out = child.wait_with_output().ok()?;
    let _ = writer.join();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(str::to_string)
}

/// Fetches whichever of `shas` the clone lacks, in batches; returns how many
/// are still missing afterwards.
pub fn fetch_missing(clone: &Path, shas: &[String]) -> usize {
    let missing: Vec<&str> = shas
        .iter()
        .map(String::as_str)
        .filter(|sha| !has_commit(clone, sha))
        .collect();
    for batch in missing.chunks(100) {
        let mut args = vec!["fetch", "-q", "origin", "--end-of-options"];
        args.extend(batch);
        // One commit the server no longer has fails the whole batch.
        if git(clone, &args).is_err() {
            for sha in batch {
                let _ = git(clone, &["fetch", "-q", "origin", "--end-of-options", sha]);
            }
        }
    }
    missing.iter().filter(|sha| !has_commit(clone, sha)).count()
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
