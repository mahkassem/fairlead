//! Where a repository hears that its setup has gaps without asking: a new
//! session, a CI plan, and `config check` on a smoke entry naming no file.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// A clean environment, so a developer's FAIRLEAD_*, CI or git settings
/// can't leak in.
fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    if let Some(value) = std::env::var_os("SYSTEMROOT") {
        cmd.env("SYSTEMROOT", value);
    }
    cmd.env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    cmd
}

/// The exit code, stdout and stderr of `fairlead` with `args` and `env`.
fn fairlead(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = command(dir, env!("CARGO_BIN_EXE_fairlead"), args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A git repository holding `files`, committed once on `main`.
fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-told-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, content) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["add", "-A"],
        &["commit", "-q", "-m", "base"],
    ] {
        let out = command(&dir, "git", args).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    dir
}

const CONFIG: &str = r#"
[tests]
match = { replace = ["**/*.test.ts"] }

[[tests.runners]]
id = "unit"
match = ["**/*.test.ts"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]
"#;

const FILES: &[(&str, &str)] = &[
    ("fairlead.toml", CONFIG),
    ("src/a.ts", "export const a = 1;\n"),
    ("src/a.test.ts", "import { a } from './a';\n"),
];

/// What the SessionStart hook adds to the session, or nothing.
fn session_start(dir: &Path) -> String {
    let call = json!({"session_id": "s", "cwd": dir, "source": "startup"}).to_string();
    let mut child = command(dir, env!("CARGO_BIN_EXE_fairlead"), &["resume", "--hook"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(call.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "the hook always exits 0");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().is_empty() {
        return String::new();
    }
    let answer: Value = serde_json::from_str(text.trim()).unwrap();
    answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_new_session_is_told_of_required_open_items_once_a_day() {
    let dir = repo("session", FILES);
    let told = session_start(&dir);
    assert!(
        told.starts_with("fairlead: setup has ") && told.contains("run `fairlead setup`"),
        "{told}"
    );
    assert!(told.contains("hooks"), "{told}");
    assert_eq!(told.lines().count(), 1, "{told}");
    assert_eq!(session_start(&dir), "", "the same day says it once");

    // Yesterday's stamp is stale, so the line comes back.
    std::fs::write(dir.join(".git/fairlead/setup-told"), "1").unwrap();
    assert_eq!(session_start(&dir), told);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_session_is_told_nothing_once_no_required_item_is_open() {
    let dir = repo("quiet", FILES);
    let (_, out, err) = fairlead(&dir, &["setup"], &[]);
    let (code, check, _) = fairlead(&dir, &["setup", "--check"], &[]);
    // Only lefthook is missing here, and that is not this machine's to require.
    assert_eq!(code, 0, "{out}{err}{check}");
    assert_eq!(session_start(&dir), "");
    assert!(
        !dir.join(".git/fairlead/setup-told").exists(),
        "nothing told, nothing stamped"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn ci_plan_warns_about_what_the_checkout_shows_and_only_that() {
    let dir = repo("ci", FILES);
    let plan = ["ci", "plan", "--base", "main", "--out", "plan.json"];
    let (code, _, err) = fairlead(&dir, &plan, &[("GITHUB_ACTIONS", "true")]);
    assert_eq!(code, 0, "{err}");
    let line = (err.lines())
        .find(|l| l.starts_with("::warning title=Fairlead setup::"))
        .unwrap_or_else(|| panic!("no warning in: {err}"));
    // No agent settings are committed, so the hooks are no business of CI's.
    assert!(line.contains("(agents)"), "{line}");
    assert!(
        line.ends_with("run `fairlead setup` and commit what it writes"),
        "{line}"
    );

    let (_, _, err) = fairlead(&dir, &plan, &[]);
    assert!(err.contains("fairlead: setup has 1 open"), "{err}");

    fairlead(&dir, &["agents", "sync"], &[]);
    let (code, _, err) = fairlead(&dir, &plan, &[("GITHUB_ACTIONS", "true")]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("Fairlead setup"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn config_check_and_setup_name_a_smoke_entry_that_matches_no_test_file() {
    let stale = format!("{CONFIG}smoke = [\"src/a.test.ts\", \"src/gone.test.ts\"]\n");
    let dir = repo(
        "smoke",
        &[
            ("fairlead.toml", stale.as_str()),
            ("src/a.ts", "export const a = 1;\n"),
            ("src/a.test.ts", "import { a } from './a';\n"),
        ],
    );
    let (code, out, err) = fairlead(&dir, &["config", "check"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        err.contains("warning: tests.runners `unit`: its smoke entry `src/gone.test.ts` names no test file the runner matches"),
        "{err}"
    );
    assert!(!err.contains("`src/a.test.ts`"), "{err}");

    let (_, out, _) = fairlead(&dir, &["setup", "--check"], &[]);
    assert!(
        out.contains("unit: its `smoke` entry `src/gone.test.ts` names no test file it matches"),
        "{out}"
    );
    assert!(
        out.contains("fix: name test files that exist under `smoke` on runner `unit`"),
        "{out}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
