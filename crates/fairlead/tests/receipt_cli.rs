//! `fairlead receipt` and `fairlead next` on a real repository: what changed
//! against the brief, the tests a file outside it adds, and the one step due
//! in each state of the change loop.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    // A clean environment, so a developer's FAIRLEAD_*, CI, git or session settings can't leak in.
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
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("CLAUDE_CODE_SESSION_ID", "s1");
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

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

fn next(dir: &Path) -> String {
    let (code, out) = fairlead(dir, &["next", "--base", "main"]);
    assert_eq!(code, 0, "{out}");
    out.trim().to_string()
}

const CONFIG: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "test ! -f FAIL", "sh", "{files}"]

[done]
guard = false
"#;

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-receipt-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join(".gitignore"), "FAIL\n").unwrap();
    std::fs::write(dir.join("src/a.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(
        dir.join("src/b.ts"),
        "import { a } from './a';\nexport const b = a;\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/auth.ts"), "export const s = 1;\n").unwrap();
    std::fs::write(
        dir.join("test/b.test.ts"),
        "import { b } from '../src/b';\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("test/auth.test.ts"),
        "import { s } from '../src/auth';\n",
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    dir
}

#[test]
fn next_names_the_one_step_due_in_every_state_of_the_loop() {
    let dir = repo("next");
    assert!(next(&dir).contains("nothing due; nothing has changed"));
    std::fs::write(dir.join("src/a.ts"), "export const a = 2;\n").unwrap();
    assert!(next(&dir).starts_with("next: brief:"), "{}", next(&dir));
    fairlead(&dir, &["brief", "--base", "main", "src/a.ts"]);
    assert!(next(&dir).starts_with("next: done:"), "{}", next(&dir));
    std::fs::write(dir.join("FAIL"), "").unwrap();
    assert_eq!(fairlead(&dir, &["done"]).0, 1);
    let failed = next(&dir);
    assert!(
        failed.contains("fix the failing step, `sh -c test ! -f FAIL sh"),
        "{failed}"
    );
    std::fs::remove_file(dir.join("FAIL")).unwrap();
    assert_eq!(fairlead(&dir, &["done"]).0, 0);
    assert!(next(&dir).starts_with("next: receipt:"), "{}", next(&dir));
    fairlead(&dir, &["receipt"]);
    assert!(next(&dir).starts_with("next: ready:"), "{}", next(&dir));
    std::fs::write(dir.join("src/a.ts"), "export const a = 3;\n").unwrap();
    assert!(
        next(&dir).starts_with("next: done:"),
        "an edit makes the pass stale"
    );
}

#[test]
fn next_opens_a_draft_pull_request_before_the_gate_and_marks_it_ready_after() {
    let dir = repo("draft");
    fairlead(&dir, &["brief", "--base", "main", "src/a.ts"]);
    std::fs::write(dir.join("src/a.ts"), "export const a = 2;\n").unwrap();
    let before = next(&dir);
    assert!(before.starts_with("next: done:"), "{before}");
    assert!(before.contains("`gh pr create --draft`"), "{before}");
    assert!(!before.contains("gh pr ready"), "{before}");
    assert_eq!(fairlead(&dir, &["done"]).0, 0);
    let after = next(&dir);
    assert!(after.starts_with("next: receipt:"), "{after}");
    assert!(after.contains("`gh pr ready`"), "{after}");
    assert!(!after.contains("--draft"), "{after}");
    let (code, out) = fairlead(&dir, &["receipt"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("next     ready:"), "{out}");
    assert!(next(&dir).contains("`gh pr ready`"), "{}", next(&dir));
}

#[test]
fn the_receipt_splits_named_reached_and_outside_with_the_tests_outside_adds() {
    let dir = repo("receipt");
    fairlead(&dir, &["brief", "--base", "main", "src/a.ts"]);
    std::fs::write(dir.join("src/a.ts"), "export const a = 2;\n").unwrap();
    std::fs::write(
        dir.join("src/b.ts"),
        "import { a } from './a';\nexport const b = a + 1;\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/auth.ts"), "export const s = 2;\n").unwrap();
    let (code, out) = fairlead(&dir, &["receipt", "--out", "receipt.json"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("3 files: 1 named in the brief, 1 in its reach, 1 outside"),
        "{out}"
    );
    assert!(
        out.contains("outside  src/auth.ts  adds 1 test (test/auth.test.ts)"),
        "{out}"
    );
    assert!(
        out.contains("planned now 2, briefed 1: 1 added by files outside the brief"),
        "{out}"
    );
    assert!(out.contains("gate     not run for this tree"), "{out}");
    let r: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(r["named"], serde_json::json!(["src/a.ts"]));
    assert_eq!(r["reached"], serde_json::json!(["src/b.ts"]));
    assert_eq!(r["outside"][0]["path"], "src/auth.ts");
    assert_eq!(
        r["outside"][0]["adds"],
        serde_json::json!(["test/auth.test.ts"])
    );
    assert!(dir.join(".git/fairlead/receipts").is_dir());
}

#[test]
fn with_no_brief_the_receipt_lists_what_changed_and_the_gate() {
    let dir = repo("nobrief");
    std::fs::write(dir.join("src/auth.ts"), "export const s = 2;\n").unwrap();
    let (code, out) = fairlead(&dir, &["receipt", "--base", "main"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("no brief for this session, so nothing to compare against"),
        "{out}"
    );
    assert!(out.contains("changed  src/auth.ts"), "{out}");
    assert!(out.contains("next     brief:"), "{out}");
}
