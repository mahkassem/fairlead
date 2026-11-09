//! `fairlead done` on a real repository: the gate's steps run, the outcome
//! is recorded against the tree, and any edit makes a pass stale.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(dir: &Path, program: &str, args: &[&str]) -> Output {
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

fn git(dir: &Path, args: &[&str]) {
    let out = run(dir, "git", args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn done(dir: &Path, args: &[&str]) -> (i32, String) {
    let mut all = vec!["done", "--base", "main"];
    all.extend(args);
    let out = run(dir, env!("CARGO_BIN_EXE_fairlead"), &all);
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

const CONFIG: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "test ! -f FAIL", "sh", "{files}"]

[[checks]]
id = "lint"
command = ["sh", "-c", "exit 0"]

[done]
always = ["lint"]
"#;

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-done-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join("a.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(dir.join("test/a.test.ts"), "import { a } from '../a';\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("a.ts"), "export const a = 2;\n").unwrap();
    dir
}

#[test]
fn a_passing_gate_is_recorded_for_the_tree_and_an_edit_makes_it_stale() {
    let dir = repo("pass");
    let (code, out) = done(&dir, &["--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("test/a.test.ts") && out.contains("exit 0") && out.contains("guard check"),
        "planned test, the always check and the guard: {out}"
    );
    assert!(out.contains("3 steps"), "{out}");
    assert_eq!(done(&dir, &["--check"]).0, 1, "nothing ran yet");
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("done: passed, 3 steps"), "{out}");
    let (code, out) = done(&dir, &["--check"]);
    assert_eq!((code, out.trim()), (0, "done: passed for this tree"));
    std::fs::write(dir.join("a.ts"), "export const a = 3;\n").unwrap();
    let (code, out) = done(&dir, &["--check"]);
    assert_eq!(
        (code, out.trim()),
        (
            1,
            "done: not passed for this tree: `fairlead done` hasn't run on the tree as it is now; run `fairlead done`"
        )
    );
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    let last: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(last["stage"], "done");
    let ids: Vec<&str> = last["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["unit", "lint", "guard"]);
}

#[test]
fn a_failing_step_stops_the_gate_and_is_recorded_as_a_failure() {
    let dir = repo("fail");
    std::fs::write(dir.join("FAIL"), "").unwrap();
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("done: failed unit"), "{out}");
    assert!(
        !out.contains("guard check"),
        "it stops at the first failure: {out}"
    );
    assert_eq!(done(&dir, &["--check"]).0, 1);
    let (code, out) = done(&dir, &["--keep-going"]);
    assert_eq!(code, 1);
    assert!(out.contains("done: passed guard"), "every step runs: {out}");
}

#[test]
fn a_failed_step_says_whether_it_could_start_and_its_first_error_line() {
    let dir = repo("why");
    let boot = "echo 'Error: connect ECONNREFUSED 127.0.0.1:5432' >&2; exit 3";
    std::fs::write(
        dir.join("fairlead.toml"),
        CONFIG.replace("test ! -f FAIL", boot),
    )
    .unwrap();
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 1, "{out}");
    let line = out
        .lines()
        .find(|l| l.starts_with("done: failed unit"))
        .unwrap();
    assert!(
        line.ends_with(
            "s: could not start (exit 3 before any test ran): Error: connect ECONNREFUSED 127.0.0.1:5432"
        ),
        "{line}"
    );
    let ran = "echo '2 passed, 1 failed'; echo 'Error: expected 2, got 3'; exit 1";
    std::fs::write(
        dir.join("fairlead.toml"),
        CONFIG.replace("test ! -f FAIL", ran),
    )
    .unwrap();
    let (_, out) = done(&dir, &[]);
    let line = out
        .lines()
        .find(|l| l.starts_with("done: failed unit"))
        .unwrap();
    assert!(
        line.ends_with("s: exit 1 after its tests ran: Error: expected 2, got 3"),
        "{line}"
    );
}

#[test]
fn done_always_must_name_a_check() {
    let dir = repo("bad");
    std::fs::write(
        dir.join("fairlead.toml"),
        CONFIG.replace("always = [\"lint\"]", "always = [\"typecheck\"]"),
    )
    .unwrap();
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 2);
    assert!(
        out.contains("done.always[0]") && out.contains("typecheck"),
        "{out}"
    );
}

/// Runs the Stop hook as Claude Code would, returning its exit code and stderr.
fn stop(dir: &Path, active: bool) -> (i32, String) {
    stop_as(dir, "s", active)
}

fn stop_as(dir: &Path, session: &str, active: bool) -> (i32, String) {
    use std::io::Write as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .args(["guard", "stop"])
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = serde_json::json!({
        "session_id": session, "cwd": dir, "hook_event_name": "Stop", "stop_hook_active": active
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// An edit by `session` as the write hook sees it, which logs it.
fn edit(dir: &Path, session: &str) {
    use std::io::Write as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .args(["guard", "hook"])
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = serde_json::json!({
        "session_id": session, "cwd": dir, "hook_event_name": "PreToolUse",
        "tool_name": "Write",
        "tool_input": { "file_path": dir.join("a.ts"), "content": "export const a = 9;\n" }
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

#[test]
fn the_stop_hook_sends_an_agent_back_until_the_tree_it_leaves_has_passed() {
    let dir = repo("stop");
    edit(&dir, "s");
    let (code, err) = stop(&dir, false);
    assert_eq!(code, 2, "a change with no gate: {err}");
    assert!(err.contains("Run `fairlead done`"), "{err}");
    assert_eq!(
        stop(&dir, true).0,
        0,
        "under ask, the second stop goes through"
    );
    assert_eq!(done(&dir, &[]).0, 0);
    assert_eq!(stop(&dir, false).0, 0, "the tree passed");
    edit(&dir, "s");
    std::fs::write(dir.join("a.ts"), "export const a = 4;\n").unwrap();
    assert_eq!(
        stop(&dir, false).0,
        2,
        "an edit after the pass makes it stale"
    );
}

#[test]
fn ask_sends_a_session_back_once_per_tree_though_each_turn_clears_stop_hook_active() {
    let dir = repo("once");
    edit(&dir, "one");
    edit(&dir, "two");
    let (code, err) = stop_as(&dir, "one", false);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains("this is the one reminder for this tree in this session"),
        "{err}"
    );
    for turn in 0..3 {
        assert_eq!(
            stop_as(&dir, "one", false).0,
            0,
            "turn {turn} on the same tree goes through"
        );
    }
    assert_eq!(stop_as(&dir, "two", false).0, 2, "another session is asked");
    std::fs::write(dir.join("a.ts"), "export const a = 5;\n").unwrap();
    assert_eq!(stop_as(&dir, "one", false).0, 2, "a new tree is asked");
    assert_eq!(stop_as(&dir, "one", false).0, 0);
}

#[test]
fn a_session_that_edited_nothing_since_the_last_pass_is_let_stop() {
    let dir = repo("nested");
    let (code, err) = stop_as(&dir, "nested", false);
    assert_eq!(code, 0, "a nested `claude -p` that only read stops: {err}");
    edit(&dir, "agent");
    assert_eq!(
        stop_as(&dir, "agent", false).0,
        2,
        "the session that edited is asked"
    );
    assert_eq!(stop_as(&dir, "nested", false).0, 0);
    assert_eq!(done(&dir, &[]).0, 0);
    std::fs::write(dir.join("a.ts"), "export const a = 6;\n").unwrap();
    assert_eq!(
        stop_as(&dir, "agent", false).0,
        0,
        "no edit by this session since the pass"
    );
    let config = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    std::fs::write(
        dir.join("fairlead.toml"),
        config.replace("[done]", "[guard]\nevents = \"off\"\n\n[done]"),
    )
    .unwrap();
    assert_eq!(
        stop_as(&dir, "unlogged", false).0,
        2,
        "with no write log it can't tell, so it asks as before"
    );
}

#[test]
fn require_keeps_sending_it_back_and_off_or_no_change_never_does() {
    let dir = repo("require");
    let config = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    std::fs::write(
        dir.join("fairlead.toml"),
        format!("{config}on_stop = \"require\"\n"),
    )
    .unwrap();
    edit(&dir, "s");
    assert_eq!(stop(&dir, true).0, 2, "require ignores stop_hook_active");
    std::fs::write(
        dir.join("fairlead.toml"),
        format!("{config}on_stop = \"off\"\n"),
    )
    .unwrap();
    assert_eq!(stop(&dir, false).0, 0);
    std::fs::write(dir.join("fairlead.toml"), &config).unwrap();
    git(&dir, &["stash", "-q", "--include-untracked"]);
    let (code, err) = stop(&dir, false);
    assert_eq!(code, 0, "nothing changed from the base: {err}");
}

#[test]
fn a_pass_in_a_linked_worktree_is_in_the_shared_log_and_outlives_the_worktree() {
    let dir = repo("worktree");
    git(&dir, &["commit", "-q", "-am", "change"]);
    let wt = dir.with_file_name(format!("fairlead-done-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt);
    git(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            wt.to_str().unwrap(),
            "change",
        ],
    );
    std::fs::write(wt.join("a.ts"), "export const a = 4;\n").unwrap();
    let (code, out) = done(&wt, &[]);
    assert_eq!(code, 0, "{out}");
    git(
        &dir,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    assert!(
        log.lines().any(|l| l.contains("\"stage\":\"done\"")),
        "the worktree's pass is in the main checkout's log: {log}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
