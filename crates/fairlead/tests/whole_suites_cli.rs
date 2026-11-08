//! Whole suites left to a later stage, through the binary: a run-all change
//! runs only the specs it reaches, a change reaching most of a suite waits
//! whole and holds `done` until `--full`, the early phase lets it pass, and
//! a scheduled run still runs every spec.

use std::path::{Path, PathBuf};
use std::process::Command;

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
    assert!(out.status.success(), "git {args:?}");
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

const RUNNERS: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]
"#;

/// Three specs: one imports src/a.ts, two import src/b.ts. `head` writes
/// the change on a branch.
fn repo(name: &str, config: &str, head: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-whole-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["src", "test", "e2e"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    let files = [
        ("fairlead.toml", config),
        (".gitignore", "*.json\n!package.json\n"),
        ("package.json", "{\"name\":\"app\",\"version\":\"1.0.0\"}\n"),
        ("src/a.ts", "export const a = 1;\n"),
        ("src/b.ts", "export const b = 1;\n"),
        ("test/a.test.ts", "import { a } from '../src/a';\n"),
        ("e2e/a.spec.ts", "import { a } from '../src/a';\n"),
        ("e2e/b.spec.ts", "import { b } from '../src/b';\n"),
        ("e2e/c.spec.ts", "import { b } from '../src/b';\n"),
    ];
    for (path, text) in files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    for (path, text) in head {
        std::fs::write(dir.join(path), text).unwrap();
    }
    git(&dir, &["commit", "-qam", "change"]);
    dir
}

fn plan_json(dir: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["plan", "--base", "main", "--json"];
    args.extend_from_slice(extra);
    let (code, out) = fairlead(dir, &args);
    assert_eq!(code, 0, "{out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"))
}

fn specs(plan: &Value) -> Vec<String> {
    plan["tests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["runner"] == "e2e")
        .map(|t| t["path"].as_str().unwrap().to_string())
        .collect()
}

fn whole(plan: &Value) -> Vec<Value> {
    plan["deferred"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d["why"] == "whole")
        .collect()
}

const BUMP: (&str, &str) = ("package.json", "{\"name\":\"app\",\"version\":\"1.0.1\"}\n");

#[test]
fn a_run_all_change_runs_only_the_specs_it_reaches_and_names_them() {
    let config = format!("{RUNNERS}all_from = \"full\"\n");
    let dir = repo(
        "reach",
        &config,
        &[BUMP, ("src/a.ts", "export const a = 2;\n")],
    );
    let plan = plan_json(&dir, &[]);
    assert_eq!(plan["all"], true);
    assert_eq!(specs(&plan), vec!["e2e/a.spec.ts"]);
    let deferred = whole(&plan);
    assert_eq!(deferred.len(), 1, "{plan}");
    assert_eq!(deferred[0]["id"], "e2e");
    assert_eq!(deferred[0]["from"], "full");
    assert_eq!(deferred[0]["selected"], 2);
    assert_eq!(plan["named"], serde_json::json!(["e2e"]));
    let e2e: Vec<&Value> = plan["invocations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["id"] == "e2e")
        .collect();
    assert_eq!(e2e.len(), 1);
    assert!(e2e[0]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a == "e2e/a.spec.ts"));
    // The unit runner still runs every test, as before.
    assert!(plan["tests"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["path"] == "test/a.test.ts"));
    // `--full` plans the whole suite, as a run here would.
    let full = plan_json(&dir, &["--full"]);
    assert_eq!(specs(&full).len(), 3);
    assert!(whole(&full).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_lockfile_change_alone_leaves_the_suite_to_the_schedule() {
    let config = format!("{RUNNERS}all_from = \"full\"\n");
    let dir = repo("bump", &config, &[BUMP]);
    let plan = plan_json(&dir, &[]);
    assert!(specs(&plan).is_empty());
    assert_eq!(whole(&plan)[0]["selected"], 3);
    assert!(whole(&plan)[0].get("reached").is_none());
    let (_, text) = fairlead(&dir, &["plan", "--base", "main"]);
    assert!(
        text.contains("e2e: the whole suite runs at the full stage"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_change_reaching_most_of_a_suite_holds_done_until_full() {
    let config = format!("{RUNNERS}all_from = \"full\"\n");
    let dir = repo("most", &config, &[("src/b.ts", "export const b = 2;\n")]);
    let plan = plan_json(&dir, &[]);
    assert_eq!(plan["all"], false);
    assert!(specs(&plan).is_empty());
    assert_eq!(whole(&plan)[0]["reached"], 2);
    let (code, out) = fairlead(&dir, &["done"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("this change reaches 2 of 3 test files (66%)"),
        "{out}"
    );
    assert!(out.contains("not finished"), "{out}");
    let (code, out) = fairlead(&dir, &["done", "--check"]);
    assert_eq!(code, 1, "{out}");
    let (code, out) = fairlead(&dir, &["done", "--full"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("not finished"), "{out}");
    let (code, out) = fairlead(&dir, &["done", "--check"]);
    assert_eq!(code, 0, "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_early_phase_defers_every_whole_suite_and_lets_done_pass() {
    let config = format!("phase = \"early\"\n{RUNNERS}");
    let dir = repo(
        "early",
        &config,
        &[BUMP, ("src/b.ts", "export const b = 2;\n")],
    );
    let plan = plan_json(&dir, &[]);
    let ids: Vec<String> = whole(&plan)
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec!["unit", "e2e"], "{plan}");
    let (code, out) = fairlead(&dir, &["done"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("not finished"), "{out}");
    let (code, _) = fairlead(&dir, &["done", "--check"]);
    assert_eq!(code, 0);
    let (_, brief) = fairlead(&dir, &["brief", "src/b.ts"]);
    assert!(
        brief.contains("phase    early: nothing is deployed"),
        "{brief}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_scheduled_run_runs_the_whole_suite_and_a_pull_request_doesnt() {
    let config = format!("{RUNNERS}all_from = \"full\"\n");
    let dir = repo("ci", &config, &[BUMP]);
    let event = dir.join("event.json");
    let ci = |name: &str, payload: &str| {
        std::fs::write(&event, payload).unwrap();
        let out = command(
            &dir,
            env!("CARGO_BIN_EXE_fairlead"),
            &["ci", "plan", "--base", "main", "--out", "plan.json"],
        )
        .env("GITHUB_ACTIONS", "true")
        .env("GITHUB_EVENT_NAME", name)
        .env("GITHUB_REF", "refs/heads/change")
        .env("GITHUB_EVENT_PATH", &event)
        .output()
        .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let plan: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("plan.json")).unwrap()).unwrap();
        (text, plan)
    };
    let (text, plan) = ci(
        "pull_request",
        "{\"pull_request\":{\"draft\":false,\"labels\":[]}}",
    );
    assert!(text.contains("stage: ready"), "{text}");
    assert!(specs(&plan).is_empty());
    assert!(
        text.contains("e2e: the whole suite runs at the full stage"),
        "{text}"
    );
    let (text, plan) = ci("schedule", "{}");
    assert!(text.contains("stage: full"), "{text}");
    assert!(whole(&plan).is_empty());
    let all = plan["invocations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["id"] == "e2e");
    assert!(all, "{plan}");
    let (_, workflow) = fairlead(&dir, &["ci", "workflow"]);
    assert!(
        workflow.contains("  schedule:\n    - cron: '17 19 * * 1-5'"),
        "{workflow}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn explain_plan_and_done_agree_on_a_spec_a_graph_edge_reaches() {
    let config = format!(
        "{RUNNERS}all_from = \"full\"\n\n[[graph.edges]]\nfrom = \"e2e/*.spec.ts\"\nto = [\"src/a.ts\"]\n"
    );
    let dir = repo("edge", &config, &[("src/a.ts", "export const a = 2;\n")]);
    let plan = plan_json(&dir, &[]);
    assert!(specs(&plan).is_empty(), "{plan}");
    assert_eq!(whole(&plan)[0]["reached"], 3, "{plan}");
    assert_eq!(whole(&plan)[0]["by_rule"], 2, "{plan}");
    let share = "e2e: this change reaches 3 of 3 test files (100%), 2 through a `[[graph.edges]]` rule, `plan.whole_share` or more, so the whole suite waits for the full stage";
    let (_, text) = fairlead(&dir, &["plan", "--base", "main"]);
    assert!(text.contains(share), "{text}");
    let (_, done) = fairlead(&dir, &["done", "--dry-run"]);
    assert!(!done.contains("e2e/b.spec.ts"), "{done}");
    let (code, why) = fairlead(
        &dir,
        &["test", "--base", "main", "--explain", "e2e/b.spec.ts"],
    );
    assert_eq!(code, 0, "{why}");
    assert!(
        why.starts_with("e2e/b.spec.ts is reached, but not run in this plan: its whole suite waits\nsrc/a.ts\n  -> e2e/b.spec.ts  ([[graph.edges]] rule)\n"),
        "{why}"
    );
    assert!(why.contains(share), "{why}");
    std::fs::remove_dir_all(&dir).unwrap();
}
