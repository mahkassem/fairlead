//! The receipt says where each step of the gate ran, by stage and
//! platform, which planned steps never ran on this tree, and which this
//! machine can't prove, such as a check that needs another OS.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(dir: &Path, program: &str, args: &[&str]) -> (i32, String) {
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
    (out.status.code().unwrap_or(-1), text)
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    run(dir, env!("CARGO_BIN_EXE_fairlead"), args)
}

/// An OS this test isn't running on.
fn elsewhere() -> &'static str {
    if std::env::consts::OS == "linux" {
        "macos"
    } else {
        "linux"
    }
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-stages-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("test")).unwrap();
    run(&dir, "git", &["init", "-q", "-b", "main"]);
    let config = format!(
        r#"[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{{files}}"]

[[checks]]
id = "lint"
command = ["sh", "-c", "test ! -f FAIL"]

[[checks]]
id = "types"
command = ["true"]

[[checks]]
id = "native-build"
command = ["true"]

[done]
always = ["lint", "types", "native-build"]
guard = false

[[needs]]
id = "other-os"
os = ["{}"]
checks = ["native-build"]
proved_in = "CI ({} job)"
"#,
        elsewhere(),
        elsewhere()
    );
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    std::fs::write(dir.join(".gitignore"), "FAIL\n").unwrap();
    std::fs::write(dir.join("test/a.test.ts"), "export {};\n").unwrap();
    run(&dir, "git", &["add", "-A"]);
    run(&dir, "git", &["commit", "-q", "-m", "base"]);
    std::fs::write(dir.join("test/a.test.ts"), "export const a = 1;\n").unwrap();
    dir
}

#[test]
fn the_receipt_says_where_each_step_ran_what_never_ran_and_what_needs_another_os() {
    let dir = repo("gaps");
    std::fs::write(dir.join("FAIL"), "").unwrap();
    let (code, out) = fairlead(&dir, &["done", "--base", "main"]);
    assert_eq!(code, 1, "{out}");
    let (code, text) = fairlead(&dir, &["receipt", "--base", "main"]);
    assert_eq!(code, 0, "{text}");
    let here = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
    let other = elsewhere();
    for line in [
        format!("ran      draft, {here}: lint (failed)\n"),
        format!("ran      ready, {here}: unit\n"),
        "not run  never run on this tree: types\n".to_string(),
        format!(
            "not run  unverifiable here: native-build needs {other}; proved in CI ({other} job)\n"
        ),
    ] {
        assert!(text.contains(&line), "missing {line:?} in\n{text}");
    }
    let (_, json) = fairlead(&dir, &["receipt", "--base", "main", "--json"]);
    let receipt: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(receipt["never_run"], serde_json::json!(["types"]));
    assert_eq!(receipt["gate"]["ran"][0]["os"], std::env::consts::OS);
    assert_eq!(receipt["unverifiable"][0]["why"], format!("needs {other}"));
    std::fs::remove_file(dir.join("FAIL")).unwrap();
    let (code, out) = fairlead(&dir, &["done", "--base", "main"]);
    assert_eq!(code, 0, "{out}");
    let (_, text) = fairlead(&dir, &["receipt", "--base", "main"]);
    assert!(!text.contains("never run"), "{text}");
    assert!(text.contains("unverifiable here: native-build"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
