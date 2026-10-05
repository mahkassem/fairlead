//! The write stage fed Codex's PreToolUse JSON, whose one editing tool is
//! `apply_patch`, and `fairlead hooks install --codex` on a real repository.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.env("HOME", dir).env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

/// A repository with `fairlead.toml` and the given files, all committed.
fn repo(name: &str, config: &str, files: &[(&str, String)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-codex-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "start"]);
    dir
}

/// Every field Codex's PreToolUse input requires, with the patch as the
/// `apply_patch` tool sends it.
fn patch_call(dir: &Path, patch: &str) -> Value {
    json!({
        "session_id": "s",
        "turn_id": "t",
        "transcript_path": null,
        "cwd": dir.to_string_lossy(),
        "hook_event_name": "PreToolUse",
        "model": "m",
        "permission_mode": "default",
        "tool_name": "apply_patch",
        "tool_use_id": "call-1",
        "tool_input": { "command": patch },
    })
}

fn hook(dir: &Path, call: Value) -> Option<Value> {
    let mut child = command(dir, &["guard", "hook"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(call.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "the hook never fails a call");
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    (!text.is_empty()).then(|| serde_json::from_str(text).unwrap())
}

fn reason(answer: &Value) -> &str {
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
    answer["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or("")
}

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

const SIZE: &str = "[guard]\nbudget_ms = 10000\n[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 3\nratchet = false\n";

#[test]
fn a_patch_is_checked_file_by_file_and_denied_for_the_one_that_adds_a_finding() {
    let dir = repo(
        "deny",
        SIZE,
        &[("src/a.ts", lines(2)), ("src/b.ts", lines(2))],
    );
    let patch = "*** Begin Patch\n*** Update File: src/a.ts\n@@\n line 2\n+line 3\n*** Update File: src/b.ts\n@@\n line 2\n+line 3\n+line 4\n*** End Patch\n";
    let answer = hook(&dir, patch_call(&dir, patch)).unwrap();
    let why = reason(&answer);
    assert!(
        why.contains("src/b.ts:1 file-length: file is 4 lines, over 3"),
        "{why}"
    );
    assert!(!why.contains("src/a.ts"), "{why}");
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    let last: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(
        (last["tool"].as_str(), last["file"].as_str()),
        (Some("apply_patch"), Some("src/b.ts"))
    );

    let added = "*** Begin Patch\n*** Add File: src/c.ts\n+one\n+two\n+three\n+four\n*** End Patch";
    assert!(reason(&hook(&dir, patch_call(&dir, added)).unwrap()).contains("src/c.ts:1"));
}

#[test]
fn a_moved_file_is_checked_where_it_lands_and_a_clean_patch_goes_ahead() {
    let dir = repo("move", SIZE, &[("src/a.ts", lines(3))]);
    let moved = "*** Begin Patch\n*** Update File: src/a.ts\n*** Move to: src/b.ts\n@@\n line 3\n+line 4\n*** End Patch";
    assert!(reason(&hook(&dir, patch_call(&dir, moved)).unwrap()).contains("src/b.ts:1"));
    let clean =
        "*** Begin Patch\n*** Update File: src/a.ts\n@@\n-line 3\n+line three\n*** End Patch";
    assert_eq!(hook(&dir, patch_call(&dir, clean)), None);
}

#[test]
fn a_patch_that_does_not_parse_or_apply_goes_ahead_for_codex_to_refuse() {
    let dir = repo("broken", SIZE, &[("src/a.ts", lines(2))]);
    let missing = "*** Begin Patch\n*** Update File: src/a.ts\n@@\n-not in the file\n+x\n+y\n+z\n*** End Patch";
    assert_eq!(hook(&dir, patch_call(&dir, missing)), None);
    assert_eq!(hook(&dir, patch_call(&dir, "*** Begin Patch\n+x")), None);
}

#[test]
fn a_patch_that_edits_deletes_or_moves_a_migration_is_denied_and_adding_one_is_not() {
    let config = "[guard]\nbudget_ms = 10000\n[guard.migrations]\nfiles = [\"db/*.sql\"]\n";
    let dir = repo("migration", config, &[("db/001.sql", "select 1;\n".into())]);
    for patch in [
        "*** Begin Patch\n*** Update File: db/001.sql\n@@\n-select 1;\n+select 2;\n*** End Patch",
        "*** Begin Patch\n*** Delete File: db/001.sql\n*** End Patch",
        "*** Begin Patch\n*** Update File: db/001.sql\n*** Move to: db/000.sql\n@@\n select 1;\n+select 0;\n*** End Patch",
    ] {
        let answer = hook(&dir, patch_call(&dir, patch)).unwrap();
        assert!(reason(&answer).contains("db/001.sql is a migration that already exists"), "{patch}");
    }
    let new = "*** Begin Patch\n*** Add File: db/002.sql\n+select 2;\n*** End Patch";
    assert_eq!(hook(&dir, patch_call(&dir, new)), None);
}

#[test]
fn install_codex_writes_codex_hooks_that_select_apply_patch_and_uninstall_removes_them() {
    let dir = repo(
        "install",
        "[[guard.commands]]\nmatch = \"rm -rf\"\nreason = \"no\"\n",
        &[],
    );
    let out = command(&dir, &["hooks", "install", "--codex"])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{said}");
    assert!(
        said.contains("Codex runs a project's hooks once the project is trusted"),
        "{said}"
    );
    assert!(
        !dir.join(".claude").exists(),
        "Claude Code's settings are left alone"
    );
    let file = dir.join(".codex/hooks.json");
    let hooks: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let pre = hooks["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre[0]["matcher"], "apply_patch|Edit|Write");
    assert_eq!(pre[1]["matcher"], "Bash");
    assert!(pre[0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("fairlead guard hook"));
    assert!(hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("fairlead guard stop"));

    let out = command(&dir, &["hooks", "uninstall", "--codex"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        !file.exists() && !dir.join(".codex").exists(),
        "install made both, so uninstall takes both"
    );
}
