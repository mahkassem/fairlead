//! `fairlead migrate` on a repository set up the way an older release left
//! it: its hooks, its config floor and its version pins.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.env("HOME", dir).env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-migrate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = command(&dir, "git", &["init", "-q"]).output().unwrap();
    assert!(out.status.success());
    dir
}

fn put(dir: &Path, path: &str, text: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

/// The settings 0.4 wrote: one guard on edits, called by name, beside a hook of the project's own.
fn old_settings() -> String {
    let own = json!({ "matcher": "Bash", "hooks": [{ "type": "command", "command": "./scripts/check-shell" }] });
    let ours = json!({ "matcher": "Edit|Write|MultiEdit", "hooks": [{ "type": "command", "command": "command -v fairlead >/dev/null 2>&1 && fairlead guard hook || true", "timeout": 10 }] });
    serde_json::to_string_pretty(&json!({ "permissions": { "allow": ["Bash(ls)"] }, "hooks": { "PreToolUse": [own, ours] } })).unwrap() + "\n"
}

fn older_repository(name: &str) -> PathBuf {
    let dir = scratch(name);
    put(&dir, ".claude/settings.json", &old_settings());
    put(
        &dir,
        "package.json",
        "{\n  \"name\": \"app\",\n  \"devDependencies\": {\n    \"fairlead\": \"^0.4.2\"\n  }\n}\n",
    );
    put(&dir, "bun.lock", "{}\n");
    put(
        &dir,
        "lefthook.yml",
        "pre-commit:\n  commands:\n    fairlead-guard:\n      run: fairlead guard check --staged\n",
    );
    put(
        &dir,
        "fairlead.toml",
        "# the project's config\nfairlead = \"0.4\"\n\n[done]\nalways = []\n",
    );
    put(
        &dir,
        ".github/workflows/ci.yml",
        "jobs:\n  plan:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - uses: mahkassem/fairlead@v0.4.2\n        with:\n          version: v0.4.2\n          command: ci plan\n",
    );
    dir
}

fn files(dir: &Path) -> Vec<String> {
    [
        ".claude/settings.json",
        "package.json",
        "lefthook.yml",
        "fairlead.toml",
        ".github/workflows/ci.yml",
    ]
    .iter()
    .map(|p| read(dir, p))
    .collect()
}

#[test]
fn a_dry_run_lists_every_update_and_writes_nothing() {
    let dir = older_repository("dry");
    let before = files(&dir);
    let (code, out) = fairlead(&dir, &["migrate"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("from ^0.4.2 (package.json)"), "{out}");
    for file in [
        ".claude/settings.json",
        "lefthook.yml",
        "fairlead.toml",
        "package.json",
        ".github/workflows/ci.yml",
    ] {
        assert!(
            out.contains(&format!("would update {file}:")),
            "{file} missing:\n{out}"
        );
    }
    assert!(
        out.contains("adds the brief nudge and the Stop hook"),
        "{out}"
    );
    assert!(out.contains("Fairlead's action at v0.4.2 becomes"), "{out}");
    assert!(out.contains("run `bun install`"), "{out}");
    assert!(out.contains("review (0.6.0)"), "{out}");
    assert!(out.contains("review (0.5.0)"), "{out}");
    assert_eq!(files(&dir), before, "a dry run wrote");
    let (code, out) = fairlead(&dir, &["migrate", "--check"]);
    assert_eq!(code, 1, "{out}");
}

#[test]
fn write_brings_everything_to_this_release_and_a_second_run_finds_nothing() {
    let dir = older_repository("write");
    let (code, out) = fairlead(&dir, &["migrate", "--write"]);
    assert_eq!(code, 0, "{out}");
    let version = env!("CARGO_PKG_VERSION");

    let settings: Value = serde_json::from_str(&read(&dir, ".claude/settings.json")).unwrap();
    assert_eq!(settings["permissions"], json!({ "allow": ["Bash(ls)"] }));
    let pre = settings["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(
        pre[0]["hooks"][0]["command"], "./scripts/check-shell",
        "the project's own hook moved"
    );
    let guard = pre[1]["hooks"][0]["command"].as_str().unwrap();
    assert!(guard.contains("bun x fairlead guard hook"), "{guard}");
    assert!(settings["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("guard stop"));
    assert!(settings["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("guard nudge"));

    assert!(read(&dir, "lefthook.yml").contains("run: bun x fairlead guard check --staged"));
    assert!(read(&dir, "fairlead.toml").starts_with("# the project's config\nfairlead = \"0.6\"\n"));
    assert!(read(&dir, "package.json").contains(&format!("\"fairlead\": \"^{version}\"")));
    let workflow = read(&dir, ".github/workflows/ci.yml");
    assert!(
        workflow.contains(&format!("mahkassem/fairlead@v{version}")),
        "{workflow}"
    );
    assert!(
        workflow.contains(&format!("version: v{version}")),
        "{workflow}"
    );

    let (code, out) = fairlead(&dir, &["migrate", "--check"]);
    assert_eq!(code, 0, "a second run still had work:\n{out}");
    assert!(out.contains("nothing to update"), "{out}");
}

#[test]
fn uninstall_after_a_migration_still_restores_the_file_from_before_fairlead() {
    let dir = older_repository("restore");
    let mut original: Value = serde_json::from_str(&old_settings()).unwrap();
    original["hooks"]["PreToolUse"]
        .as_array_mut()
        .unwrap()
        .pop();
    let original = serde_json::to_string_pretty(&original).unwrap() + "\n";
    let record = json!({ "original": original, "written": old_settings(), "made_dir": false });
    put(
        &dir,
        ".git/fairlead/backups/claude-settings.json",
        &record.to_string(),
    );
    let (code, out) = fairlead(&dir, &["migrate", "--write"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = fairlead(&dir, &["hooks", "uninstall", "--claude"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("byte for byte"), "{out}");
    assert_eq!(read(&dir, ".claude/settings.json"), original);
}

#[test]
fn a_repository_on_this_release_has_nothing_to_update() {
    let dir = scratch("current");
    put(
        &dir,
        "fairlead.toml",
        "fairlead = \"0.6\"\n\n[done]\nalways = []\n",
    );
    let (code, out) = fairlead(&dir, &["hooks", "install", "--claude"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = fairlead(&dir, &["migrate", "--check"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nothing to update"), "{out}");
    assert!(out.contains("from 0.6 (the config's floor)"), "{out}");
    assert!(
        !out.contains("review (0.5.0)"),
        "notes older than the floor showed:\n{out}"
    );
}
