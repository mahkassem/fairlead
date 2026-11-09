//! The coach in `fairlead done`: a cheap check CI history shows red is
//! added and runs first, with a line saying why; with no history nothing
//! changes. `done --pre-push` runs the plan and the push checks, and only
//! the push checks once the tree has passed; `hooks install --git` moves an
//! earlier release's push entry onto it.

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

const CONFIG: &str = r#"[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]

[[checks]]
id = "typecheck"
command = ["sh", "-c", "exit 0"]
paths = ["types/**"]
cheap = true

[[checks]]
id = "e2e"
command = ["sh", "-c", "exit 0"]
paths = ["e2e/**"]

[done]
guard = false
"#;

fn repo(name: &str, extra: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-coach-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), format!("{CONFIG}{extra}")).unwrap();
    std::fs::write(dir.join("test/a.test.ts"), "export {};\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    std::fs::write(dir.join("test/a.test.ts"), "export const a = 1;\n").unwrap();
    dir
}

/// A `ci run --results` file in which `failed` went red.
fn results(dir: &Path, name: &str, failed: &[&str]) {
    let folder = dir.join(".git/fairlead/results");
    std::fs::create_dir_all(&folder).unwrap();
    let ran: Vec<serde_json::Value> = ["typecheck", "e2e", "unit"]
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id, "kind": if *id == "unit" { "runner" } else { "check" },
                "cwd": ".", "argv": [id], "passed": !failed.contains(id),
                "seconds": if *id == "e2e" { 600.0 } else { 2.0 }
            })
        })
        .collect();
    let file = serde_json::json!({
        "version": 1, "plan_id": "p", "passed": failed.is_empty(), "seconds": 6.0,
        "invocations": ran
    });
    std::fs::write(folder.join(format!("{name}.json")), file.to_string()).unwrap();
}

#[test]
fn a_cheap_check_red_in_ci_is_added_first_and_says_why() {
    let dir = repo("added", "");
    let (ok, out) = fairlead(&dir, &["done", "--base", "main", "--dry-run"]);
    assert!(ok, "{out}");
    assert!(!out.contains("typecheck"), "no history, no change: {out}");
    results(&dir, "r1", &["typecheck", "e2e"]);
    results(&dir, "r2", &["typecheck"]);
    results(&dir, "r3", &[]);
    let (ok, out) = fairlead(&dir, &["done", "--base", "main"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("done: typecheck: added, red in CI 2 of the last 3 runs"),
        "{out}"
    );
    assert!(
        !out.contains("e2e"),
        "e2e isn't cheap and wasn't planned: {out}"
    );
    let typecheck = out.find("(.) sh -c exit 0\n").unwrap();
    let unit = out.find("(.) sh -c exit 0 sh test/a.test.ts").unwrap();
    assert!(typecheck < unit, "the red check runs first: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn before_a_push_the_plan_runs_once_then_only_the_push_checks() {
    let dir = repo("push", "\n[pr]\nup_to_date = false\n");
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    git(&dir, &["commit", "-q", "-am", "change"]);
    let (ok, out) = fairlead(&dir, &["done", "--base", "main", "--pre-push"]);
    assert!(ok, "{out}");
    assert!(out.contains("done: passed unit"), "{out}");
    assert!(out.contains("guard check --push"), "{out}");
    assert!(!out.contains("pr check --local"), "{out}");
    let (ok, out) = fairlead(&dir, &["done", "--base", "main", "--pre-push"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("done: passed for this tree already, so only the push checks run"),
        "{out}"
    );
    assert!(!out.contains("passed unit"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hooks_install_moves_an_earlier_push_entry_onto_done_pre_push() {
    let dir = repo("hook", "\n[pr]\nup_to_date = false\n");
    let old = "pre-commit:\n  commands:\n    fairlead-guard:\n      run: fairlead guard check --staged\npre-push:\n  commands:\n    fairlead-push:\n      run: fairlead guard check --push\n";
    std::fs::write(dir.join("lefthook.yml"), old).unwrap();
    let (ok, out) = fairlead(&dir, &["hooks", "install", "--git"]);
    assert!(ok, "{out}");
    let now = std::fs::read_to_string(dir.join("lefthook.yml")).unwrap();
    assert!(
        now.contains("      run: fairlead done --pre-push\n"),
        "{now}"
    );
    assert_eq!(now.matches("fairlead-push").count(), 1, "{now}");
    let _ = std::fs::remove_dir_all(&dir);
}
