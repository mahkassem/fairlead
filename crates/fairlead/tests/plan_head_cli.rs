//! `fairlead plan --head REV` and `test --explain --head REV`: a past commit
//! planned from a temporary checkout of it, with its own config, while the
//! working tree is elsewhere and dirty, and the checkout gone afterwards.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    // A clean environment, so a developer's FAIRLEAD_*, CI or git settings can't leak in.
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT", "TMPDIR", "TEMP", "TMP"] {
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
    cmd
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = command(dir, "git", args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fairlead(dir: &Path, args: &[&str]) -> Output {
    command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap()
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

const CONFIG: &str = "[[tests.runners]]\nid = \"unit\"\nmatch = [\"test/**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n";

struct Fixture {
    dir: PathBuf,
    init: String,
    first: String,
}

/// Three commits on main: the base, one changing `src/a.ts`, and one
/// changing `src/b.ts` and renaming the runner. The working tree is left
/// dirty, with an edit to the config and an untracked file.
fn fixture(name: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("fairlead-head-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", CONFIG);
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(&dir, "src/b.ts", "export const b = 1;\n");
    write(&dir, "test/a.test.ts", "import { a } from '../src/a';\n");
    write(&dir, "test/b.test.ts", "import { b } from '../src/b';\n");
    let init = commit(&dir, "init");
    write(&dir, "src/a.ts", "export const a = 2;\n");
    let first = commit(&dir, "first");
    write(&dir, "src/b.ts", "export const b = 2;\n");
    write(
        &dir,
        "fairlead.toml",
        &CONFIG.replace("\"unit\"", "\"renamed\""),
    );
    commit(&dir, "second");
    write(&dir, "fairlead.toml", "this isn't toml [");
    write(&dir, "src/untracked.ts", "export const u = 1;\n");
    Fixture { dir, init, first }
}

fn worktrees(dir: &Path) -> usize {
    git(dir, &["worktree", "list", "--porcelain"])
        .lines()
        .filter(|l| l.starts_with("worktree "))
        .count()
}

fn plan_json(dir: &Path, args: &[&str]) -> Value {
    let mut all = vec!["plan", "--json"];
    all.extend(args);
    let out = fairlead(dir, &all);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("fairlead-head-"),
        "no temporary path: {stdout}"
    );
    serde_json::from_str(&stdout).unwrap()
}

#[test]
fn a_past_commit_plans_what_it_needed_with_its_own_config() {
    let f = fixture("past");
    let plan = plan_json(&f.dir, &["--head", &f.first]);
    assert_eq!(
        plan["changed"],
        json!([{"path": "src/a.ts", "status": "modified"}])
    );
    assert_eq!(plan["tests"].as_array().unwrap().len(), 1, "{plan}");
    assert_eq!(plan["tests"][0]["path"], "test/a.test.ts");
    assert_eq!(plan["head"], f.first.as_str(), "the head is the commit");
    assert_eq!(plan["base"], f.init.as_str(), "on main, its parent");
    assert_eq!(
        plan["invocations"][0]["id"], "unit",
        "the config at the commit"
    );
    assert_eq!(worktrees(&f.dir), 1, "the temporary worktree is gone");

    let explicit = plan_json(&f.dir, &["--head", &f.first, "--base", "HEAD~2"]);
    assert_eq!(explicit["base"], f.init.as_str(), "--base is read here");
    assert_eq!(explicit["changed"], plan["changed"]);

    let text = fairlead(&f.dir, &["plan", "--head", &f.first[..7]]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("test/a.test.ts") && !text.contains("test/b.test.ts"),
        "{text}"
    );
}

#[test]
fn a_commit_off_the_default_branch_is_measured_from_its_merge_base() {
    let f = fixture("branch");
    git(&f.dir, &["stash", "-u", "-q"]);
    git(&f.dir, &["checkout", "-q", "-b", "side", &f.init]);
    write(&f.dir, "src/b.ts", "export const b = 3;\n");
    let side = commit(&f.dir, "side");
    git(&f.dir, &["checkout", "-q", "main"]);
    let plan = plan_json(&f.dir, &["--head", "side"]);
    assert_eq!(plan["head"], side.as_str());
    assert_eq!(plan["base"], f.init.as_str());
    assert_eq!(plan["tests"][0]["path"], "test/b.test.ts");
}

#[test]
fn explain_reads_the_plan_of_the_head() {
    let f = fixture("explain");
    let explain = |target: &str| {
        let out = fairlead(&f.dir, &["test", "--head", &f.first, "--explain", target]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let yes = explain("test/a.test.ts");
    assert!(
        yes.contains("is selected (unit)") && yes.contains("src/a.ts"),
        "{yes}"
    );
    assert!(explain("test/b.test.ts").contains("isn't selected"));
    let from_below = fairlead(
        &f.dir.join("test"),
        &["test", "--head", &f.first, "--explain", "a.test.ts"],
    );
    assert!(String::from_utf8_lossy(&from_below.stdout).contains("is selected"));
    assert_eq!(worktrees(&f.dir), 1);
}

#[test]
fn a_bad_head_fails_cleanly_and_leaves_no_worktree() {
    let f = fixture("bad");
    let fails = |args: &[&str], says: &str| {
        let out = fairlead(&f.dir, args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {err}");
        assert!(err.contains(says), "{args:?}: {err}");
    };
    fails(&["plan", "--head", "no-such-rev"], "isn't a commit");
    fails(
        &["plan", "--head", &f.first, "--files", "src/a.ts"],
        "--files",
    );
    fails(&["plan", "--head", &f.init], "no parent");
    fails(
        &["plan", "--head", &f.first, "--base", "nope"],
        "isn't a commit",
    );
    assert_eq!(worktrees(&f.dir), 1);
}

#[cfg(unix)]
#[test]
fn a_worktree_an_interrupted_run_left_is_removed_by_the_next() {
    let f = fixture("leftover");
    let left = std::env::temp_dir().join(format!("fairlead-head-999999999-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&left);
    git(
        &f.dir,
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            &left.to_string_lossy(),
            &f.first,
        ],
    );
    assert_eq!(worktrees(&f.dir), 2);
    plan_json(&f.dir, &["--head", &f.first]);
    assert_eq!(worktrees(&f.dir), 1);
    assert!(!left.exists());
}
