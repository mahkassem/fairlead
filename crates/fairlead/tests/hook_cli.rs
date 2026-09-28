//! The write stage and `fairlead hooks`, on real git repositories: the hook
//! fed Claude Code's PreToolUse JSON on stdin, and install and uninstall on
//! settings files that already hold other hooks.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
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
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let out = command(dir, "git", args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fairlead(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    text(out)
}

fn text(out: Output) -> (bool, String, String) {
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Runs the hook on one call and returns its answer, if it gave one.
fn hook(dir: &Path, call: Value) -> Option<Value> {
    let mut child = command(dir, env!("CARGO_BIN_EXE_fairlead"), &["guard", "hook"])
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
    let (ok, out, err) = text(child.wait_with_output().unwrap());
    assert!(ok, "the hook never fails a call: {err}");
    let out = out.trim();
    (!out.is_empty()).then(|| serde_json::from_str(out).unwrap())
}

fn call(dir: &Path, tool: &str, input: Value) -> Value {
    json!({
        "session_id": "s",
        "cwd": dir.to_string_lossy(),
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": input,
    })
}

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

/// A repository with `fairlead.toml` and the given files, all committed.
fn repo(name: &str, config: &str, files: &[(&str, String)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-hook-{name}-{}", std::process::id()));
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

/// Tests judge decisions, not speed, so a slow machine's process start can't
/// push a call past the budget and let it through.
const BUDGET: &str = "[guard]\nbudget_ms = 10000\n";
const SIZE: &str = "[guard]\nbudget_ms = 10000\n[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 3\nratchet = false\n";

fn events(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn reason(answer: &Value) -> &str {
    answer["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or("")
}

#[test]
fn a_write_that_adds_a_finding_is_denied_with_it_and_logged() {
    let dir = repo("deny", SIZE, &[("src/a.ts", lines(2))]);
    let path = dir.join("src/a.ts");
    let answer = hook(
        &dir,
        call(
            &dir,
            "Write",
            json!({"file_path": path, "content": lines(4)}),
        ),
    )
    .unwrap();
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(
        reason(&answer).contains("src/a.ts:1 file-length: file is 4 lines, over 3"),
        "{answer}"
    );
    let last = events(&dir).pop().unwrap();
    assert_eq!(
        (last["stage"].as_str(), last["decision"].as_str()),
        (Some("write"), Some("deny"))
    );
    assert_eq!(last["file"], "src/a.ts");
    assert_eq!(last["tool"], "Write");
    assert!(
        !last.to_string().contains("line 4"),
        "the log never holds content: {last}"
    );
}

#[test]
fn an_edit_that_adds_nothing_old_debt_included_goes_ahead_silently() {
    let dir = repo("allow", SIZE, &[("src/a.ts", lines(5))]);
    let path = dir.join("src/a.ts");
    let edit = json!({"file_path": path, "old_string": "line 2", "new_string": "line two"});
    assert_eq!(hook(&dir, call(&dir, "Edit", edit)), None);
    assert_eq!(events(&dir).pop().unwrap()["decision"], "allow");
}

#[test]
fn an_edit_that_cannot_be_rebuilt_or_an_unknown_shape_goes_ahead() {
    let dir = repo("unknown", SIZE, &[("src/a.ts", lines(2))]);
    let path = dir.join("src/a.ts");
    let missing = json!({"file_path": path, "old_string": "absent", "new_string": &lines(9)});
    assert_eq!(hook(&dir, call(&dir, "Edit", missing)), None);
    assert_eq!(
        hook(
            &dir,
            call(&dir, "Edit", json!({"file_path": path, "patch": "x"}))
        ),
        None
    );
    let multi = json!({"file_path": path, "edits": [
        {"old_string": "line 1", "new_string": "a\nb\nc\nd"},
        {"old_string": "line 2", "new_string": "e"}
    ]});
    assert!(
        hook(&dir, call(&dir, "MultiEdit", multi)).is_some(),
        "a rebuilt multi-edit is checked"
    );
}

#[test]
fn warn_adds_a_note_and_never_grants_permission() {
    let config = SIZE.replace("[guard]\n", "[guard]\non_finding = \"warn\"\n");
    let dir = repo("warn", &config, &[("src/a.ts", lines(2))]);
    let path = dir.join("src/a.ts");
    let answer = hook(
        &dir,
        call(
            &dir,
            "Write",
            json!({"file_path": path, "content": lines(4)}),
        ),
    )
    .unwrap();
    let out = &answer["hookSpecificOutput"];
    assert!(out.get("permissionDecision").is_none(), "{answer}");
    assert!(out["additionalContext"]
        .as_str()
        .unwrap()
        .contains("CI will fail on them"));
    assert_eq!(events(&dir).pop().unwrap()["decision"], "warn");
}

#[test]
fn a_new_file_in_a_new_directory_is_checked_like_any_other() {
    let dir = repo("newdir", SIZE, &[]);
    let path = dir.join("src/new/deeper/a.ts");
    let answer = hook(
        &dir,
        call(
            &dir,
            "Write",
            json!({"file_path": path, "content": lines(4)}),
        ),
    )
    .unwrap();
    assert!(
        reason(&answer).contains("src/new/deeper/a.ts:1 file-length"),
        "{answer}"
    );
}

#[test]
fn a_command_rule_denies_a_shell_command_with_its_reason() {
    let config = format!("{SIZE}[[guard.commands]]\nmatch = '(^|\\s)git push --force(\\s|$)'\nreason = \"Open a pull request instead.\"\n");
    let dir = repo("bash", &config, &[]);
    let answer = hook(
        &dir,
        call(&dir, "Bash", json!({"command": "git push --force"})),
    )
    .unwrap();
    assert_eq!(
        reason(&answer),
        "fairlead guard: Open a pull request instead."
    );
    assert_eq!(
        hook(&dir, call(&dir, "Bash", json!({"command": "git status"}))),
        None
    );
}

#[test]
fn editing_a_migration_that_exists_is_denied_and_adding_one_is_not() {
    let config = format!("{BUDGET}[guard.migrations]\nfiles = [\"db/*.sql\"]\n");
    let dir = repo(
        "migration",
        &config,
        &[("db/001.sql", "select 1;\n".into())],
    );
    let existing =
        json!({"file_path": dir.join("db/001.sql"), "old_string": "1", "new_string": "2"});
    let answer = hook(&dir, call(&dir, "Edit", existing)).unwrap();
    assert!(
        reason(&answer).contains("db/001.sql is a migration that already exists"),
        "{answer}"
    );
    let new = json!({"file_path": dir.join("db/002.sql"), "content": "select 2;\n"});
    assert_eq!(hook(&dir, call(&dir, "Write", new)), None);
}

#[test]
fn no_rules_bad_input_another_tool_or_a_path_outside_the_project_all_go_ahead() {
    let dir = repo("quiet", "", &[("src/a.ts", lines(2))]);
    let write = json!({"file_path": dir.join("src/a.ts"), "content": lines(9)});
    assert_eq!(hook(&dir, call(&dir, "Write", write)), None);
    assert!(
        events(&dir).is_empty(),
        "a project with no rules logs nothing"
    );

    let dir = repo("outside", SIZE, &[]);
    let outside = std::env::temp_dir().join("elsewhere.ts");
    assert_eq!(
        hook(
            &dir,
            call(
                &dir,
                "Write",
                json!({"file_path": outside, "content": lines(9)})
            )
        ),
        None
    );
    assert_eq!(
        hook(
            &dir,
            call(&dir, "Read", json!({"file_path": dir.join("src/a.ts")}))
        ),
        None
    );
    let mut child = command(&dir, env!("CARGO_BIN_EXE_fairlead"), &["guard", "hook"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"not json").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success() && out.stdout.is_empty());
}

#[test]
fn a_file_larger_than_the_hook_reads_goes_ahead() {
    let dir = repo("large", SIZE, &[("src/a.ts", lines(2))]);
    let big = "x\n".repeat(200 * 1024);
    let write = json!({"file_path": dir.join("src/a.ts"), "content": big});
    assert_eq!(hook(&dir, call(&dir, "Write", write)), None);
}

/// A settings file with a hook of its own and formatting `serde_json` wouldn't write.
const THEIRS: &str = "{\n\t\"model\": \"x\",\n\t\"hooks\": {\"PreToolUse\": [{\"matcher\": \"Bash\", \"hooks\": [{\"type\": \"command\", \"command\": \"./check.sh\"}]}]}\n}";

fn settings(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(".claude").join(name)).unwrap()
}

#[test]
fn uninstall_restores_an_existing_settings_file_byte_for_byte_in_either_place() {
    for (flag, name) in [
        ("--shared", "settings.json"),
        ("--local", "settings.local.json"),
    ] {
        let dir = repo(
            &format!("roundtrip{flag}"),
            SIZE,
            &[(&*format!(".claude/{name}"), THEIRS.to_string())],
        );
        let (ok, out, err) = fairlead(&dir, &["hooks", "install", flag]);
        assert!(ok, "{err}");
        assert!(out.contains("installed"), "{out}");
        let after: Value = serde_json::from_str(&settings(&dir, name)).unwrap();
        let groups = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "theirs is kept and ours added: {after}");
        assert_eq!(after["model"], "x");
        let (_, out, _) = fairlead(&dir, &["hooks", "status", flag]);
        assert!(out.contains("byte for byte"), "{out}");
        let (ok, out, _) = fairlead(&dir, &["hooks", "uninstall", flag]);
        assert!(ok && out.contains("byte for byte"), "{out}");
        assert_eq!(settings(&dir, name), THEIRS);
    }
}

#[test]
fn install_without_a_settings_file_makes_one_and_uninstall_takes_it_away() {
    let dir = repo("fresh", SIZE, &[]);
    assert!(fairlead(&dir, &["hooks", "install"]).0);
    let after: Value = serde_json::from_str(&settings(&dir, "settings.json")).unwrap();
    assert_eq!(
        after["hooks"]["PreToolUse"][0]["matcher"],
        "Edit|Write|MultiEdit"
    );
    assert!(after["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("fairlead guard hook"));
    assert_eq!(
        after["hooks"]["PreToolUse"].as_array().unwrap().len(),
        1,
        "no Bash hook without command rules"
    );
    assert!(fairlead(&dir, &["hooks", "uninstall"]).0);
    assert!(!dir.join(".claude").exists());
}

#[test]
fn install_is_idempotent_and_hooks_bash_only_with_command_rules() {
    let config = format!(
        "{SIZE}[[guard.commands]]\nmatch = \"x\"\nreason = \"y\"\n[hooks]\nclaude = \"local\"\n"
    );
    let dir = repo("twice", &config, &[]);
    assert!(fairlead(&dir, &["hooks", "install"]).0);
    let first = settings(&dir, "settings.local.json");
    let (ok, out, _) = fairlead(&dir, &["hooks", "install"]);
    assert!(ok && out.contains("already installed"), "{out}");
    assert_eq!(settings(&dir, "settings.local.json"), first);
    let after: Value = serde_json::from_str(&first).unwrap();
    let matchers: Vec<&str> = after["hooks"]["PreToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["matcher"].as_str().unwrap())
        .collect();
    assert_eq!(matchers, ["Edit|Write|MultiEdit", "Bash"]);
}

#[test]
fn a_file_changed_since_install_keeps_everything_but_the_guard() {
    let dir = repo(
        "changed",
        SIZE,
        &[(".claude/settings.json", THEIRS.to_string())],
    );
    assert!(fairlead(&dir, &["hooks", "install"]).0);
    let mut edited: Value = serde_json::from_str(&settings(&dir, "settings.json")).unwrap();
    edited["permissions"] = json!({"allow": ["Bash(ls)"]});
    std::fs::write(dir.join(".claude/settings.json"), edited.to_string()).unwrap();
    let (_, out, _) = fairlead(&dir, &["hooks", "status"]);
    assert!(out.contains("changed since install"), "{out}");
    let (ok, out, _) = fairlead(&dir, &["hooks", "uninstall"]);
    assert!(ok && out.contains("formatting may differ"), "{out}");
    let left: Value = serde_json::from_str(&settings(&dir, "settings.json")).unwrap();
    assert_eq!(left["permissions"]["allow"][0], "Bash(ls)");
    assert_eq!(left["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(
        left["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "./check.sh"
    );
}

#[test]
fn a_fresh_clone_with_no_manifest_removes_the_guard_by_its_command() {
    let dir = repo(
        "clone",
        SIZE,
        &[(".claude/settings.json", THEIRS.to_string())],
    );
    assert!(fairlead(&dir, &["hooks", "install"]).0);
    std::fs::remove_dir_all(dir.join(".git/fairlead/backups")).unwrap();
    let (ok, out, _) = fairlead(&dir, &["hooks", "uninstall"]);
    assert!(ok && out.contains("removed Fairlead's entries"), "{out}");
    let left: Value = serde_json::from_str(&settings(&dir, "settings.json")).unwrap();
    assert!(!left.to_string().contains("fairlead guard hook"));
    assert_eq!(left["model"], "x");
}

#[test]
fn a_settings_file_that_is_not_json_is_left_alone() {
    let dir = repo(
        "broken",
        SIZE,
        &[(".claude/settings.json", "{ not json".into())],
    );
    let (ok, _, err) = fairlead(&dir, &["hooks", "install"]);
    assert!(!ok);
    assert!(err.contains("isn't valid JSON"), "{err}");
    assert_eq!(settings(&dir, "settings.json"), "{ not json");
}

const LEFTHOOK: &str = "# Shared git hooks.\npre-commit:\n  parallel: true\n  commands:\n    # lint first\n    lint:\n      run: npm run lint\n";

fn file(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

#[test]
fn install_adds_the_commit_stage_to_an_existing_lefthook_config_and_uninstall_restores_it() {
    let dir = repo("lefthook", SIZE, &[("lefthook.yml", LEFTHOOK.to_string())]);
    let (ok, out, err) = fairlead(&dir, &["hooks", "install"]);
    assert!(ok, "{err}");
    assert!(
        out.contains("added the commit stage to") && out.contains("lefthook install"),
        "{out}"
    );
    let added = file(&dir, "lefthook.yml").unwrap();
    assert!(
        added.contains("    fairlead-guard:\n      run: fairlead guard check --staged\n"),
        "{added}"
    );
    assert!(
        added.contains("# lint first") && added.starts_with("# Shared git hooks."),
        "comments kept: {added}"
    );
    assert!(
        dir.join(".claude/settings.json").exists(),
        "the Claude hook goes in too"
    );
    let (_, out, _) = fairlead(&dir, &["hooks", "status", "--git"]);
    assert!(
        out.contains("byte for byte") && out.contains("won't run"),
        "{out}"
    );
    let (ok, out, _) = fairlead(&dir, &["hooks", "uninstall"]);
    assert!(ok && out.contains("back as it was"), "{out}");
    assert_eq!(file(&dir, "lefthook.yml").unwrap(), LEFTHOOK);
    assert!(!dir.join(".claude").exists());
}

#[test]
fn without_git_install_leaves_a_repository_with_no_lefthook_config_alone() {
    let dir = repo("nolefthook", SIZE, &[]);
    assert!(fairlead(&dir, &["hooks", "install"]).0);
    assert!(!dir.join("lefthook.yml").exists());
    let (ok, _, _) = fairlead(&dir, &["hooks", "install", "--git"]);
    assert!(ok);
    let made = file(&dir, "lefthook.yml").unwrap();
    assert!(
        made.starts_with("pre-commit:\n  commands:\n    fairlead-guard:"),
        "{made}"
    );
    assert!(fairlead(&dir, &["hooks", "uninstall", "--git"]).0);
    assert!(
        !dir.join("lefthook.yml").exists(),
        "a config install made is taken away"
    );
    assert!(
        dir.join(".claude/settings.json").exists(),
        "--git leaves the Claude hook"
    );
}

#[test]
fn a_one_line_pre_commit_is_refused_and_left_as_it_was() {
    let dir = repo(
        "inline",
        SIZE,
        &[("lefthook.yml", "pre-commit: {}\n".into())],
    );
    let (ok, _, err) = fairlead(&dir, &["hooks", "install", "--git"]);
    assert!(!ok);
    assert!(err.contains("by hand"), "{err}");
    assert_eq!(file(&dir, "lefthook.yml").unwrap(), "pre-commit: {}\n");
}

#[test]
fn doctor_reports_the_hooks_and_what_the_event_log_recorded() {
    let dir = repo("doctor", SIZE, &[]);
    assert!(fairlead(&dir, &["hooks", "install", "--git"]).0);
    std::fs::create_dir_all(dir.join(".git/hooks")).unwrap();
    std::fs::write(
        dir.join(".git/hooks/pre-commit"),
        "#!/bin/sh\nlefthook run pre-commit\n",
    )
    .unwrap();
    let mut log: Vec<String> = (1..=9)
        .map(|i| {
            json!({"stage": "write", "decision": "allow", "ms": i as f64, "rules": []}).to_string()
        })
        .collect();
    log.push(
        json!({"stage": "write", "decision": "deny", "ms": 20.0, "rules": ["file-length"]})
            .to_string(),
    );
    log.push(json!({"stage": "commit", "decision": "allow", "ms": 5.0, "rules": []}).to_string());
    std::fs::create_dir_all(dir.join(".git/fairlead")).unwrap();
    std::fs::write(
        dir.join(".git/fairlead/events.jsonl"),
        log.join("\n") + "\n",
    )
    .unwrap();
    let (ok, out, err) = fairlead(&dir, &["doctor"]);
    assert!(ok, "{err}");
    let tail: Vec<&str> = out
        .lines()
        .skip_while(|l| !l.starts_with("claude hook"))
        .filter(|l| !l.starts_with("on PATH"))
        .collect();
    assert_eq!(
        tail,
        [
            "claude hook: not installed in .claude/settings.json; `fairlead hooks install` adds it",
            "git hook: installed in lefthook.yml, and lefthook runs it",
            "write hook: 10 call(s): 9 allow, 1 deny",
            "  latency: p50 5.0 ms, p95 20.0 ms",
            "  denied by: 1 file-length",
            "commit stage: 1 run(s): 1 allow",
        ]
    );
}

#[cfg(unix)]
#[test]
fn a_package_dependency_gets_hooks_that_run_the_projects_own_copy() {
    let package = r#"{"devDependencies": {"fairlead": "^0.4.0"}}"#;
    let dir = repo(
        "package",
        SIZE,
        &[
            ("package.json", package.to_string()),
            ("bun.lock", String::new()),
            ("lefthook.yml", LEFTHOOK.to_string()),
            ("src/a.ts", lines(2)),
        ],
    );
    let (ok, _, err) = fairlead(&dir, &["hooks", "install"]);
    assert!(ok, "{err}");
    let added = file(&dir, "lefthook.yml").unwrap();
    assert!(
        added.contains("      run: bun x fairlead guard check --staged\n"),
        "{added}"
    );
    let (_, out, _) = fairlead(&dir, &["hooks", "status", "--git"]);
    assert!(out.contains("the commit stage is in"), "{out}");
    let (_, out, _) = fairlead(&dir, &["doctor"]);
    assert!(
        out.contains("the project's own copy, through `bun x`"),
        "{out}"
    );

    let after: Value = serde_json::from_str(&settings(&dir, "settings.json")).unwrap();
    let shell = after["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(shell.contains("bun x fairlead guard hook"), "{shell}");
    // Where the package has unpacked its binary, the hook calls it directly.
    let unpacked = dir.join("node_modules/fairlead/node_modules/.bin_real");
    std::fs::create_dir_all(&unpacked).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_fairlead"), unpacked.join("fairlead")).unwrap();
    let path = dir.join("src/a.ts");
    let input = call(
        &dir,
        "Write",
        json!({"file_path": path, "content": lines(4)}),
    );
    let mut child = command(&dir, "sh", &["-c", &shell])
        .env("CLAUDE_PROJECT_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let (ok, out, _) = text(child.wait_with_output().unwrap());
    assert!(ok, "the command always exits 0");
    let answer: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");

    assert!(fairlead(&dir, &["hooks", "uninstall"]).0);
    assert_eq!(file(&dir, "lefthook.yml").unwrap(), LEFTHOOK);
}
