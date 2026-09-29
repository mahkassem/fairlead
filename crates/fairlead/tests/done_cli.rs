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
    assert_eq!((code, out.trim()), (1, "done: not passed for this tree"));
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
        "session_id": "s", "cwd": dir, "hook_event_name": "Stop", "stop_hook_active": active
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

#[test]
fn the_stop_hook_sends_an_agent_back_until_the_tree_it_leaves_has_passed() {
    let dir = repo("stop");
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
    std::fs::write(dir.join("a.ts"), "export const a = 4;\n").unwrap();
    assert_eq!(
        stop(&dir, false).0,
        2,
        "an edit after the pass makes it stale"
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
