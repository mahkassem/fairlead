//! The write stage fed Gemini CLI's BeforeTool JSON, whose tools are
//! `write_file`, `replace` and `run_shell_command` and whose answer is a
//! top-level decision, and `fairlead hooks install --gemini`.

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
    let dir = std::env::temp_dir().join(format!("fairlead-gemini-{name}-{}", std::process::id()));
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

/// Gemini CLI's BeforeTool input, as its hook runner sends it.
fn before_tool(dir: &Path, tool: &str, input: Value) -> Value {
    json!({
        "session_id": "s",
        "transcript_path": "/tmp/t.json",
        "cwd": dir.to_string_lossy(),
        "hook_event_name": "BeforeTool",
        "timestamp": "2026-09-30T00:00:00.000Z",
        "tool_name": tool,
        "tool_input": input,
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

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

const SIZE: &str = "[guard]\nbudget_ms = 10000\n[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 3\nratchet = false\n";

#[test]
fn a_gemini_write_or_replace_that_adds_a_finding_is_denied_in_geminis_shape() {
    let dir = repo("deny", SIZE, &[("src/a.ts", lines(2))]);
    let write = before_tool(
        &dir,
        "write_file",
        json!({"file_path": "src/a.ts", "content": lines(4)}),
    );
    let answer = hook(&dir, write).unwrap();
    assert_eq!(answer["decision"], "deny", "{answer}");
    let reason = answer["reason"].as_str().unwrap();
    assert!(
        reason.contains("src/a.ts:1 file-length: file is 4 lines, over 3"),
        "{reason}"
    );
    assert!(answer.get("hookSpecificOutput").is_none(), "{answer}");

    let grow = json!({"file_path": dir.join("src/a.ts"), "old_string": "line", "new_string": "line\nmore", "allow_multiple": true});
    let answer = hook(&dir, before_tool(&dir, "replace", grow)).unwrap();
    assert_eq!(answer["decision"], "deny", "{answer}");

    let same = json!({"file_path": "src/a.ts", "old_string": "line 2", "new_string": "line two"});
    assert_eq!(hook(&dir, before_tool(&dir, "replace", same)), None);
}

#[test]
fn a_warning_reaches_the_person_as_a_system_message_and_a_shell_rule_denies() {
    let config = SIZE.replacen("[guard]\n", "[guard]\non_finding = \"warn\"\n", 1);
    let dir = repo("warn", &config, &[("src/a.ts", lines(2))]);
    let write = before_tool(
        &dir,
        "write_file",
        json!({"file_path": "src/a.ts", "content": lines(4)}),
    );
    let answer = hook(&dir, write).unwrap();
    assert!(
        answer["systemMessage"]
            .as_str()
            .unwrap()
            .contains("file-length"),
        "{answer}"
    );
    assert!(
        answer.get("decision").is_none(),
        "a warning never decides: {answer}"
    );

    let rules = format!("{SIZE}[[guard.commands]]\nmatch = '(^|\\s)git push --force(\\s|$)'\nreason = \"Open a pull request instead.\"\n");
    let dir = repo("shell", &rules, &[]);
    let push = before_tool(
        &dir,
        "run_shell_command",
        json!({"command": "git push --force"}),
    );
    let answer = hook(&dir, push).unwrap();
    assert_eq!(answer["decision"], "deny");
    assert!(answer["reason"]
        .as_str()
        .unwrap()
        .contains("Open a pull request instead."));
}

#[test]
fn install_gemini_merges_into_its_settings_with_its_event_names_and_milliseconds() {
    let rules = "[[guard.commands]]\nmatch = \"rm -rf\"\nreason = \"no\"\n";
    let dir = repo(
        "install",
        rules,
        &[(
            ".gemini/settings.json",
            "{\n  \"theme\": \"GitHub\"\n}\n".into(),
        )],
    );
    let before = std::fs::read_to_string(dir.join(".gemini/settings.json")).unwrap();
    let out = command(&dir, &["hooks", "install", "--gemini"])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{said}");
    assert!(said.contains("trusted folder"), "{said}");
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".gemini/settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["theme"], "GitHub", "other settings are kept");
    let hooks = &settings["hooks"];
    assert_eq!(hooks["BeforeTool"][0]["matcher"], "write_file|replace");
    assert_eq!(hooks["BeforeTool"][0]["hooks"][0]["timeout"], 10000);
    assert_eq!(hooks["BeforeTool"][1]["matcher"], "run_shell_command");
    assert_eq!(hooks["AfterAgent"][0]["hooks"][0]["timeout"], 30000);
    assert!(hooks["AfterAgent"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("fairlead guard stop"));
    assert!(hooks["AfterTool"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("fairlead guard nudge"));
    assert!(hooks.get("PreToolUse").is_none() && hooks.get("Stop").is_none());

    let out = command(&dir, &["hooks", "status", "--gemini"])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("AfterTool") && said.contains("AfterAgent"),
        "{said}"
    );
    let out = command(&dir, &["hooks", "uninstall", "--gemini"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        std::fs::read_to_string(dir.join(".gemini/settings.json")).unwrap(),
        before
    );
}
