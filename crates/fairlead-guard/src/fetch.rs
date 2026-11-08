//! A fresh copy of the base branch before migration numbers are compared
//! with it. Parallel branches each take the next free number against the
//! base they last fetched, so the comparison is only as good as that fetch.
//! The network never blocks: a fetch that fails or runs past its time is a
//! warning, and the local ref is used.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// Long enough for a fetch of one branch on a working network.
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// At a commit, a fetch newer than this is fresh enough.
pub const FRESH: Duration = Duration::from_secs(600);

/// A remote branch and the ref that tracks it here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub remote: String,
    pub branch: String,
    pub tracking: String,
}

fn git_out(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Where `base` comes from: `origin/main` names its remote; a plain `main`
/// is `origin`'s; with no base, the remote's default branch, else `main`.
pub fn remote_for(root: &Path, base: Option<&str>) -> Remote {
    let remotes: Vec<String> = git_out(root, &["remote"])
        .map(|t| t.lines().map(String::from).collect())
        .unwrap_or_default();
    let base = base.map(String::from).unwrap_or_else(|| {
        git_out(
            root,
            &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
        )
        .unwrap_or_else(|| "origin/main".into())
    });
    split(&base, &remotes)
}

fn split(base: &str, remotes: &[String]) -> Remote {
    let base = base.strip_prefix("refs/remotes/").unwrap_or(base);
    for r in remotes {
        if let Some(branch) = base.strip_prefix(&format!("{r}/")) {
            return Remote {
                remote: r.clone(),
                branch: branch.to_string(),
                tracking: base.to_string(),
            };
        }
    }
    let branch = base.strip_prefix("refs/heads/").unwrap_or(base);
    Remote {
        remote: "origin".into(),
        branch: branch.to_string(),
        tracking: format!("origin/{branch}"),
    }
}

/// When the tracking ref was last fetched: FETCH_HEAD, which every fetch
/// rewrites, or the ref's reflog, whichever is newer.
fn last_fetch(root: &Path, remote: &Remote) -> Option<SystemTime> {
    let dir = crate::git::git_dir(root)?;
    let common = crate::git::common_of(&dir);
    let reflog = common.join("logs/refs/remotes").join(&remote.tracking);
    [dir.join("FETCH_HEAD"), common.join("FETCH_HEAD"), reflog]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok()?.modified().ok())
        .max()
}

/// Runs `command`, killing it once `timeout` has passed. None when it
/// couldn't start, failed, or ran out of time.
pub fn run_with_timeout(mut command: Command, timeout: Duration) -> Option<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success().then_some(()),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Fetches the base unless `always` is false and the last fetch is fresh.
/// Returns a warning when the fetch didn't happen and the ref may be stale.
pub fn refresh(root: &Path, remote: &Remote, always: bool) -> Option<String> {
    if !always {
        let fresh = last_fetch(root, remote)
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < FRESH);
        if fresh {
            return None;
        }
    }
    let mut git = Command::new("git");
    git.args([
        "fetch",
        "--quiet",
        "--no-tags",
        &remote.remote,
        &remote.branch,
    ])
    .current_dir(root)
    .env("GIT_TERMINAL_PROMPT", "0");
    match run_with_timeout(git, TIMEOUT) {
        Some(()) => None,
        None => Some(format!(
            "migrations: couldn't fetch {} {} within {} s (offline?); comparing with the local {}, which may be stale",
            remote.remote,
            remote.branch,
            TIMEOUT.as_secs(),
            remote.tracking
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_names_its_remote_or_is_origins() {
        let remotes = vec!["origin".to_string(), "upstream".to_string()];
        let r = split("upstream/release/2", &remotes);
        assert_eq!(
            (r.remote.as_str(), r.branch.as_str(), r.tracking.as_str()),
            ("upstream", "release/2", "upstream/release/2")
        );
        let r = split("main", &remotes);
        assert_eq!(
            (r.remote.as_str(), r.tracking.as_str()),
            ("origin", "origin/main")
        );
        let r = split("refs/remotes/origin/dev", &remotes);
        assert_eq!(r.branch, "dev");
    }

    #[test]
    fn a_command_past_its_timeout_is_killed_and_reported() {
        if cfg!(windows) {
            return;
        }
        let mut sleep = Command::new("sh");
        sleep.args(["-c", "sleep 5"]);
        let start = Instant::now();
        assert!(run_with_timeout(sleep, Duration::from_millis(200)).is_none());
        assert!(start.elapsed() < Duration::from_secs(2));
        let mut ok = Command::new("sh");
        ok.args(["-c", "exit 0"]);
        assert!(run_with_timeout(ok, Duration::from_secs(2)).is_some());
    }
}
