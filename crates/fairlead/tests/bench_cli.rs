//! `fairlead guard compare` and `fairlead guard bench` on real git repositories.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn command(dir: &Path, program: &str, args: &[&str]) -> Output {
    // A clean environment, so a developer's FAIRLEAD_*, CI or git settings can't leak in.
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    cmd.output().unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = command(dir, "git", args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fairlead(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

/// Tests judge decisions, not speed.
const CONFIG: &str = "[guard]\nbudget_ms = 10000\n[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 3\nratchet = false\n";

fn repo(name: &str, files: &[(&str, String)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "start"]);
    dir
}

#[test]
fn compare_passes_when_the_other_linter_agrees_and_lists_each_difference_when_not() {
    let dir = repo(
        "compare",
        &[("src/long.ts", lines(5)), ("src/ok.ts", lines(1))],
    );
    std::fs::write(
        dir.join("theirs.txt"),
        "src/long.ts:1 max-lines: too long\n",
    )
    .unwrap();
    let (ok, out) = fairlead(
        &dir,
        &[
            "guard",
            "compare",
            "theirs.txt",
            "--map",
            "max-lines=file-length",
        ],
    );
    assert!(ok, "{out}");
    assert!(out.contains("the same, finding for finding"), "{out}");
    std::fs::write(dir.join("theirs.txt"), "src/ok.ts:1 file-length\n").unwrap();
    let (ok, out) = fairlead(&dir, &["guard", "compare", "theirs.txt"]);
    assert!(!ok);
    assert!(
        out.contains("1 only the other linter:\n  src/ok.ts:1 file-length"),
        "{out}"
    );
    assert!(
        out.contains("1 only Fairlead:\n  src/long.ts:1 file-length"),
        "{out}"
    );
    let (ok, _) = fairlead(
        &dir,
        &["guard", "compare", "theirs.txt", "--rules", "density"],
    );
    assert!(ok, "only the named rules are compared");
}

#[test]
fn bench_replays_each_commits_files_through_the_hook_with_todays_config() {
    let dir = repo(
        "bench",
        &[("src/a.ts", lines(1)), ("README.md", "x\n".into())],
    );
    let start = git(&dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("src/a.ts"), lines(2)).unwrap();
    std::fs::write(dir.join("README.md"), "y\n").unwrap();
    git(&dir, &["commit", "-qam", "small"]);
    std::fs::write(dir.join("src/a.ts"), lines(9)).unwrap();
    std::fs::write(dir.join("src/b.ts"), lines(1)).unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "grow"]);
    // The config at those commits said nothing about this rule; today's does.
    std::fs::write(
        dir.join("fairlead.toml"),
        CONFIG.replace("file_lines = 3", "file_lines = 4"),
    )
    .unwrap();
    let (ok, out) = fairlead(&dir, &["guard", "bench", "--since", &start]);
    assert!(ok, "{out}");
    assert!(out.contains("3 edit(s) from 2 commit(s)"), "{out}");
    assert!(out.contains("decisions: 2 allow, 1 deny"), "{out}");
    assert!(out.contains("p95"), "{out}");
    assert!(
        !dir.join(".git/fairlead/events.jsonl").exists(),
        "the bench logs to its own worktree"
    );
    let worktrees = git(&dir, &["worktree", "list"]);
    assert_eq!(
        worktrees.lines().count(),
        1,
        "the bench's worktree is gone: {worktrees}"
    );
    let (ok, out) = fairlead(
        &dir,
        &["guard", "bench", "--since", &start, "--p95-under", "0.001"],
    );
    assert!(!ok && out.contains("is over"), "{out}");
    let (ok, out) = fairlead(&dir, &["guard", "bench", "--since", "HEAD"]);
    assert!(ok && out.contains("no commit since"), "{out}");
}
