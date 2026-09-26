//! `fairlead plan` and `fairlead test --explain` against a real git repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fairlead_in(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.output().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

/// A committed repository with a runner, a check and two tests.
fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-plan-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", "[[tests.runners]]\nid = \"vitest\"\nmatch = [\"**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n\n[[checks]]\nid = \"typecheck\"\ncommand = [\"tsc\"]\npaths = [\"**/*.ts\"]\n");
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(&dir, "src/b.ts", "export const b = 1;\n");
    write(&dir, "test/a.test.ts", "import { a } from '../src/a';\n");
    write(&dir, "test/b.test.ts", "import { b } from '../src/b';\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

#[test]
fn plan_reads_the_changes_from_git_and_prints_the_selection() {
    let dir = project("git");
    write(&dir, "src/a.ts", "export const a = 2;\n");
    let out = fairlead_in(&dir, &["plan", "--base", "main", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["version"], 1);
    assert_eq!(plan["changed"][0]["path"], "src/a.ts");
    assert_eq!(plan["tests"].as_array().unwrap().len(), 1);
    assert_eq!(plan["tests"][0]["path"], "test/a.test.ts");
    assert_eq!(plan["head"], "worktree");
    assert!(plan["tree_hash"].as_str().unwrap().starts_with("worktree:"));
    assert_eq!(
        plan["invocations"][0]["argv"],
        serde_json::json!(["vitest", "run", "test/a.test.ts"])
    );
    let text = fairlead_in(&dir, &["plan", "--base", "main"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("test/a.test.ts"));
}

#[test]
fn a_clean_checkout_reports_its_commit_and_tree() {
    let dir = project("clean");
    let out = fairlead_in(&dir, &["plan", "--base", "main", "--json"]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["head"].as_str().unwrap().len(), 40);
    assert_eq!(plan["tree_hash"].as_str().unwrap().len(), 40);
    assert!(plan["tests"].as_array().unwrap().is_empty());
}

#[test]
fn a_base_with_no_merge_base_fails_instead_of_planning_nothing() {
    let dir = project("no-base");
    let out = fairlead_in(&dir, &["plan", "--base", "origin/not-there"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no merge base"));
}

#[test]
fn explain_gives_the_chain_for_a_selected_test_and_the_gap_for_another() {
    let dir = project("explain");
    write(&dir, "src/a.ts", "export const a = 3;\n");
    let yes = fairlead_in(
        &dir,
        &["test", "--base", "main", "--explain", "test/a.test.ts"],
    );
    let yes = String::from_utf8_lossy(&yes.stdout).to_string();
    assert!(
        yes.contains("is selected (unit)") && yes.contains("-> test/a.test.ts"),
        "{yes}"
    );
    let no = fairlead_in(
        &dir,
        &["test", "--base", "main", "--explain", "test/b.test.ts"],
    );
    assert!(String::from_utf8_lossy(&no.stdout).contains("isn't selected"));
    let check = fairlead_in(&dir, &["test", "--base", "main", "--explain", "typecheck"]);
    assert!(String::from_utf8_lossy(&check.stdout).contains("check typecheck is selected"));
}

#[test]
fn explicit_files_plan_without_git() {
    let dir = project("files");
    let out = fairlead_in(&dir.join("src"), &["plan", "--files", "b.ts", "--json"]);
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["changed"][0]["path"], "src/b.ts");
    assert_eq!(plan["tests"][0]["path"], "test/b.test.ts");
}

#[test]
fn plan_schema_prints_the_committed_schema() {
    let dir = project("schema");
    let out = fairlead_in(&dir, &["plan", "--schema"]);
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["invocations"].is_object());
}

#[test]
fn config_check_names_test_files_no_runner_matches() {
    let dir = project("runner-check");
    write(
        &dir,
        "fairlead.toml",
        "[[tests.runners]]\nid = \"vitest\"\nmatch = [\"test/a*\"]\ncommand = [\"vitest\"]\n",
    );
    let out = fairlead_in(&dir, &["config", "check"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr)
        .contains("test/b.test.ts: no [[tests.runners]] matches it"));
}

#[test]
fn explicit_files_resolve_dot_dot_and_expand_directories() {
    let dir = project("files-norm");
    let up = fairlead_in(
        &dir.join("test"),
        &["plan", "--files", "../src/a.ts", "--json"],
    );
    let plan: serde_json::Value = serde_json::from_slice(&up.stdout).unwrap();
    assert_eq!(plan["changed"][0]["path"], "src/a.ts");
    assert_eq!(plan["tests"][0]["path"], "test/a.test.ts");
    let folder = fairlead_in(&dir, &["plan", "--files", "src", "--json"]);
    let plan: serde_json::Value = serde_json::from_slice(&folder.stdout).unwrap();
    assert_eq!(plan["changed"].as_array().unwrap().len(), 2);
    assert_eq!(plan["tests"].as_array().unwrap().len(), 2);
    let outside = fairlead_in(&dir, &["plan", "--files", "../../elsewhere.ts"]);
    assert_eq!(outside.status.code(), Some(2));
    let everything = fairlead_in(&dir, &["plan", "--files", ".", "--json"]);
    let plan: serde_json::Value = serde_json::from_slice(&everything.stdout).unwrap();
    assert!(plan["changed"].as_array().unwrap().len() >= 5);
}
