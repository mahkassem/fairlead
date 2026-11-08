//! `ci ledger --rerun`: for failures no history explains, run each failing
//! test file again on HEAD; a pass makes it flaky. One that fails again runs
//! on the merge base with the base branch, in a temporary worktree: failing
//! there too makes it inherited, passing makes it the change's own. Each
//! file runs at most once on each side, under one time cap for them all.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fairlead_core::plan::InvocationKind;
use fairlead_replay::ledger::{Ledger, Verdict};

use crate::plan_cmd::{make, Changes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ran {
    Passed,
    Failed,
    /// No runner claims the file, or it isn't in that tree.
    NoTest,
    /// The time cap ran out first.
    OutOfTime,
}

/// Runs the runner invocations a plan of `file` alone selects, in `dir`,
/// each killed at the deadline.
fn run_file(dir: &Path, file: &str, deadline: Instant) -> Ran {
    if !dir.join(file).is_file() {
        return Ran::NoTest;
    }
    let Ok(planned) = make(dir, &Changes::new(None, vec![file.to_string()], Vec::new())) else {
        return Ran::NoTest;
    };
    let runs: Vec<_> = planned
        .plan
        .invocations
        .iter()
        .filter(|i| i.kind == InvocationKind::Runner)
        .collect();
    if runs.is_empty() {
        return Ran::NoTest;
    }
    for inv in runs {
        let Some((program, args)) = inv.argv.split_first() else {
            return Ran::NoTest;
        };
        let mut cmd: Command = fairlead_core::process::command(program, &dir.join(&inv.cwd));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match within(&mut cmd, deadline) {
            Ran::Passed => {}
            other => return other,
        }
    }
    Ran::Passed
}

/// Runs `cmd` until it exits or the deadline passes, when it's killed.
fn within(cmd: &mut Command, deadline: Instant) -> Ran {
    if Instant::now() >= deadline {
        return Ran::OutOfTime;
    }
    let Ok(mut child) = cmd.spawn() else {
        return Ran::Failed;
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ran::Passed,
            Ok(Some(_)) | Err(_) => return Ran::Failed,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Ran::OutOfTime;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A detached worktree at a commit, removed when dropped.
struct Temporary {
    root: PathBuf,
    path: PathBuf,
}

impl Temporary {
    fn at(root: &Path, sha: &str) -> Option<Temporary> {
        let path = std::env::temp_dir().join(format!(
            "fairlead-rerun-{}-{}",
            std::process::id(),
            &sha[..sha.len().min(12)]
        ));
        let _ = std::fs::remove_dir_all(&path);
        git(
            root,
            &["worktree", "add", "-q", "--detach", &path.to_string_lossy(), sha],
        )?;
        Some(Temporary {
            root: root.to_path_buf(),
            path,
        })
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = git(
            &self.root,
            &["worktree", "remove", "--force", &self.path.to_string_lossy()],
        );
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The merge base of HEAD with the base branch, from the remote's copy
/// when there is one.
fn merge_base(root: &Path, branch: &str) -> Option<String> {
    [format!("origin/{branch}"), branch.to_string()]
        .iter()
        .find_map(|b| git(root, &["merge-base", "HEAD", b]))
}

/// Reruns the own failures with a test file and says what each showed; the
/// verdicts change in place.
pub fn rerun(cwd: &Path, l: &mut Ledger, seconds: u64, most: usize) -> Vec<String> {
    let root = crate::graph_cmd::repo_root(cwd);
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut files: Vec<String> = Vec::new();
    for f in &l.failures {
        if let (Verdict::Own, Some(file)) = (f.verdict, &f.file) {
            if !files.contains(file) {
                files.push(file.clone());
            }
        }
    }
    let mut lines = Vec::new();
    if files.len() > most {
        lines.push(format!(
            "rerunning {most} of {} test files; --rerun-tests raises it",
            files.len()
        ));
        files.truncate(most);
    }
    let base = merge_base(&root, &l.base_branch);
    let mut worktree: Option<Option<Temporary>> = None;
    for file in files {
        let head = run_file(&root, &file, deadline);
        let (verdict, said) = match head {
            Ran::Passed => (Verdict::Flaky, "passed on a rerun of HEAD".to_string()),
            Ran::NoTest => (Verdict::Own, "no runner runs it here, so it wasn't rerun".to_string()),
            Ran::OutOfTime => (Verdict::Own, "not rerun: the time cap ran out".to_string()),
            Ran::Failed => {
                let wt = worktree.get_or_insert_with(|| {
                    base.as_deref().and_then(|b| Temporary::at(&root, b))
                });
                let short: String = base.as_deref().unwrap_or("").chars().take(7).collect();
                match wt.as_ref().map(|t| run_file(&t.path, &file, deadline)) {
                    Some(Ran::Failed) => (
                        Verdict::Inherited,
                        format!("failed again on HEAD and on the merge base {short} too"),
                    ),
                    Some(Ran::Passed) => (
                        Verdict::Own,
                        format!("failed again on HEAD and passed on the merge base {short}: a regression"),
                    ),
                    Some(Ran::NoTest) => (
                        Verdict::Own,
                        format!("failed again on HEAD; the merge base {short} doesn't have the test"),
                    ),
                    Some(Ran::OutOfTime) => (
                        Verdict::Own,
                        "failed again on HEAD; the time cap ran out before the merge base".into(),
                    ),
                    None => (
                        Verdict::Own,
                        format!("failed again on HEAD; no merge base with {}", l.base_branch),
                    ),
                }
            }
        };
        lines.push(format!("{file}: {said}"));
        for f in l
            .failures
            .iter_mut()
            .filter(|f| f.verdict == Verdict::Own && f.file.as_deref() == Some(&file))
        {
            f.verdict = verdict;
            f.rerun = Some(said.clone());
        }
    }
    lines
}
