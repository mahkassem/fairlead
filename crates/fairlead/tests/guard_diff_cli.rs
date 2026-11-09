//! A `[[guard.external]]` rule with `diff = true` counts only the findings
//! a change adds: the fake linter runs on the base too, in a temporary
//! worktree, and a finding that only moved, or was there before, doesn't count.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(dir: &Path, program: &str, args: &[&str]) -> (bool, String) {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    let out = cmd
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), text)
}

fn git(dir: &Path, args: &[&str]) {
    let (ok, out) = run(dir, "git", args);
    assert!(ok, "git {args:?}: {out}");
}

fn fairlead(dir: &Path, args: &[&str]) -> (bool, String) {
    run(dir, env!("CARGO_BIN_EXE_fairlead"), args)
}

/// A linter that prints `file:line found BAD (line N)` for each line holding BAD.
const LINT: &str = r#"for f in "$@"; do grep -n BAD "$f" | while IFS=: read -r n rest; do echo "$f:$n found BAD (line $n)"; done; done; true
"#;

fn repo(name: &str, command: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-diff-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/deep")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    let config = format!(
        "[[guard.external]]\nid = \"badword\"\ncommand = {command}\nstages = [\"commit\", \"check\"]\ndiff = true\n"
    );
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    std::fs::write(dir.join("lint.sh"), LINT).unwrap();
    std::fs::write(dir.join("src/deep/a.txt"), "BAD legacy\nok\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    dir
}

fn worktrees(dir: &Path) -> usize {
    let (_, out) = run(dir, "git", &["worktree", "list"]);
    out.lines().count()
}

#[test]
fn at_commit_a_moved_finding_passes_and_a_new_one_fails() {
    if cfg!(windows) {
        return;
    }
    let dir = repo("commit", r#"["sh", "lint.sh", "{files}"]"#);
    std::fs::write(
        dir.join("src/deep/a.txt"),
        "new top\nmore\nBAD legacy\nok\n",
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, out) = fairlead(&dir, &["guard", "check", "--staged"]);
    assert!(ok, "the legacy finding only moved: {out}");
    std::fs::write(
        dir.join("src/deep/a.txt"),
        "BAD again\nmore\nBAD legacy\nok\n",
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, out) = fairlead(&dir, &["guard", "check", "--staged"]);
    assert!(!ok, "{out}");
    assert_eq!(out.matches("found BAD").count(), 1, "{out}");
    assert_eq!(worktrees(&dir), 1, "the base worktree is removed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn at_check_only_with_a_base_and_the_whole_tree_compared_with_it() {
    if cfg!(windows) {
        return;
    }
    let all = r#"["sh", "-c", "sh lint.sh $(git ls-files src 2>/dev/null || find src -type f)"]"#;
    let dir = repo("check", all);
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("src/b.txt"), "fine\nBAD new\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "adds a finding"]);
    let (ok, out) = fairlead(&dir, &["guard", "check", "--base", "main"]);
    assert!(!ok, "{out}");
    assert!(out.contains("src/b.txt:2"), "{out}");
    assert!(!out.contains("src/deep/a.txt"), "{out}");
    let (ok, out) = fairlead(&dir, &["guard", "check"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("a diff rule runs at check only with --base"),
        "{out}"
    );
    let (ok, out) = fairlead(&dir, &["done", "--base", "main"]);
    assert!(!ok, "done passes the base to the guard: {out}");
    assert_eq!(worktrees(&dir), 1, "the base worktree is removed");
    let _ = std::fs::remove_dir_all(&dir);
}
