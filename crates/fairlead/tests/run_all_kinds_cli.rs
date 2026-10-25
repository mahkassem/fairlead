//! `plan.run_all` entries limited to kinds of change, planned from a real git
//! repository: an added migration goes through the graph, an edited one runs
//! everything and says which kind did.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
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

fn fairlead(dir: &Path, args: &[&str]) -> Output {
    command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap()
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

const RUNNER: &str = "[[tests.runners]]\nid = \"unit\"\nmatch = [\"test/**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n\n[[graph.edges]]\nfrom = \"test/db.test.ts\"\nto = [\"migrations/**\"]\n";

const KINDS: &str = "[plan]\nrun_all = [{ paths = \"migrations/**\", on = [\"modified\", \"deleted\", \"renamed\"] }]\n";

/// A committed repository with one migration, a test that depends on the
/// migrations through an edge rule, and an unrelated test; `plan` is
/// appended to the config.
fn project(name: &str, plan: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-run-all-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", &format!("{RUNNER}\n{plan}"));
    write(&dir, "tsconfig.json", "{}\n");
    write(
        &dir,
        "migrations/001_users.sql",
        "create table users (id int primary key, name text not null);\n",
    );
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(&dir, "test/a.test.ts", "import { a } from '../src/a';\n");
    write(&dir, "test/db.test.ts", "export {};\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn plan_of(dir: &Path) -> Value {
    git(dir, &["add", "-A"]);
    let out = fairlead(dir, &["plan", "--base", "main", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn tests(plan: &Value) -> Vec<&str> {
    plan["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap())
        .collect()
}

const ADDED: (&str, &str) = (
    "migrations/002_posts.sql",
    "create table posts (id int primary key);\n",
);

#[test]
fn an_added_file_under_a_kind_limited_entry_goes_through_the_graph() {
    let dir = project("added", KINDS);
    write(&dir, ADDED.0, ADDED.1);
    let plan = plan_of(&dir);
    assert_eq!(plan["changed"][0]["status"], "added");
    assert_eq!(plan["all"], false);
    assert_eq!(tests(&plan), ["test/db.test.ts"]);
    assert_eq!(plan["tests"][0]["reason"]["kind"], "import");
}

#[test]
fn a_modified_file_runs_everything_and_names_the_kind() {
    let dir = project("modified", KINDS);
    write(
        &dir,
        "migrations/001_users.sql",
        "create table users (id int primary key, name text);\n",
    );
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], true);
    assert_eq!(tests(&plan), ["test/a.test.ts", "test/db.test.ts"]);
    assert_eq!(
        plan["tests"][0]["reason"],
        serde_json::json!({ "kind": "run-all", "path": "migrations/001_users.sql", "change": "modified" })
    );
    let text = fairlead(&dir, &["plan", "--base", "main"]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("run_all (modified): migrations/001_users.sql"),
        "{text}"
    );
    let explain = fairlead(
        &dir,
        &["test", "--explain", "test/a.test.ts", "--base", "main"],
    );
    let explain = String::from_utf8_lossy(&explain.stdout);
    assert!(
        explain.contains("run_all (modified): migrations/001_users.sql"),
        "{explain}"
    );
}

#[test]
fn a_deleted_file_runs_everything_when_on_names_deleted_and_not_otherwise() {
    let dir = project("deleted", KINDS);
    std::fs::remove_file(dir.join("migrations/001_users.sql")).unwrap();
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], true);
    assert_eq!(plan["tests"][0]["reason"]["change"], "deleted");

    let only_edits = "[plan]\nrun_all = [{ paths = [\"migrations/**\"], on = [\"modified\"] }]\n";
    let dir = project("deleted-not-listed", only_edits);
    std::fs::remove_file(dir.join("migrations/001_users.sql")).unwrap();
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], false);
    assert_eq!(tests(&plan), ["test/db.test.ts"]);
}

#[test]
fn a_rename_matches_on_its_old_path_or_its_new_one() {
    let dir = project("renamed-in", KINDS);
    git(
        &dir,
        &[
            "mv",
            "migrations/001_users.sql",
            "migrations/001_create_users.sql",
        ],
    );
    let plan = plan_of(&dir);
    assert_eq!(plan["changed"][0]["status"], "renamed");
    assert_eq!(plan["all"], true);
    assert_eq!(plan["tests"][0]["reason"]["change"], "renamed");

    let dir = project("renamed-out", KINDS);
    std::fs::create_dir_all(dir.join("archive")).unwrap();
    git(
        &dir,
        &["mv", "migrations/001_users.sql", "archive/001_users.sql"],
    );
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], true);
    assert_eq!(
        plan["tests"][0]["reason"]["path"],
        "migrations/001_users.sql"
    );

    let only_edits = "[plan]\nrun_all = [{ paths = \"migrations/**\", on = [\"modified\"] }]\n";
    let dir = project("renamed-not-listed", only_edits);
    git(
        &dir,
        &[
            "mv",
            "migrations/001_users.sql",
            "migrations/001_create_users.sql",
        ],
    );
    assert_eq!(plan_of(&dir)["all"], false);
}

#[test]
fn kind_limited_entries_append_to_the_defaults_and_replace_drops_them() {
    let dir = project("defaults", KINDS);
    write(&dir, "tsconfig.json", "{ \"compilerOptions\": {} }\n");
    write(&dir, ADDED.0, ADDED.1);
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], true, "a default glob still runs everything");
    assert_eq!(
        plan["tests"][0]["reason"],
        serde_json::json!({ "kind": "run-all", "path": "tsconfig.json" })
    );

    // `warn` keeps a file nothing reaches from widening the plan by itself.
    let replaced = "[tests]\nunreached = \"warn\"\n\n[plan]\nrun_all = { replace = [\"ci/**\", { paths = \"migrations/**\", on = [\"modified\"] }] }\n";
    let dir = project("replaced", replaced);
    write(&dir, "tsconfig.json", "{ \"compilerOptions\": {} }\n");
    write(&dir, ADDED.0, ADDED.1);
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], false, "{plan}");
    assert_eq!(tests(&plan), ["test/db.test.ts"]);
    write(&dir, "ci/build.sh", "echo\n");
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], true);
    assert_eq!(
        plan["tests"][0]["reason"],
        serde_json::json!({ "kind": "run-all", "path": "ci/build.sh" })
    );
}

#[test]
fn a_plain_glob_plans_as_before_whatever_the_kind() {
    let dir = project("plain", "[plan]\nrun_all = [\"migrations/**\"]\n");
    write(&dir, ADDED.0, ADDED.1);
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], true);
    assert_eq!(
        plan["tests"][0]["reason"],
        serde_json::json!({ "kind": "run-all", "path": "migrations/002_posts.sql" })
    );
    let text = fairlead(&dir, &["plan", "--base", "main"]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("run_all: migrations/002_posts.sql"), "{text}");
}

#[test]
fn the_ci_report_names_the_kind() {
    let dir = project("report", KINDS);
    write(&dir, "migrations/001_users.sql", "drop table users;\n");
    git(&dir, &["add", "-A"]);
    let out = fairlead(
        &dir,
        &["ci", "plan", "--base", "main", "--out", "plan.json"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let summary = dir.join("summary.md");
    let out = command(
        &dir,
        env!("CARGO_BIN_EXE_fairlead"),
        &["ci", "report", "--plan", "plan.json"],
    )
    .env("GITHUB_STEP_SUMMARY", &summary)
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let md = std::fs::read_to_string(&summary).unwrap();
    assert!(
        md.contains("| a `plan.run_all` path changed (modified) | 2 |"),
        "{md}"
    );
}

#[test]
fn config_check_rejects_an_unknown_kind_an_empty_on_and_no_paths() {
    let bad = "[plan]\nrun_all = { replace = [{ paths = \"migrations/**\", on = [\"edited\"] }, { paths = \"db/**\", on = [] }, { paths = [], on = [\"added\"] }, { paths = \"x/[\", on = [\"added\"] }] }\n";
    let dir = project("check", bad);
    let out = fairlead(&dir, &["config", "check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    for line in [
        "plan.run_all[0].on[0]: `edited` isn't a kind of change; use `added`, `modified`, `deleted` or `renamed`",
        "plan.run_all[1].on: names no kind of change",
        "plan.run_all[2].paths: needs at least one glob",
        "plan.run_all[3].paths: `x/[`",
    ] {
        assert!(err.contains(line), "{line}\n{err}");
    }
    let out = fairlead(&dir, &["plan", "--base", "main", "--json"]);
    assert!(!out.status.success(), "a plan refuses an unknown kind");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("plan.run_all[0].on[0]: `edited`"), "{err}");

    let dir = project("check-ok", KINDS);
    let out = fairlead(&dir, &["config", "check"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
