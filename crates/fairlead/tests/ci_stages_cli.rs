//! CI stages through the binary: the stage read from a GitHub event, the
//! plan cut down to what runs at it, the step outputs, `ci run --only`, and
//! a config without stages planning exactly as before.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    // A clean environment, so a developer's FAIRLEAD_*, CI, git or GitHub settings can't leak in.
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

fn run(cmd: &mut Command) -> (i32, String) {
    let out = cmd.output().unwrap();
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

[[checks]]
id = "desktop-build"
command = ["sh", "-c", "exit 0"]
paths = ["src/**"]
"#;

const STAGED: &str = r#"
[stages]
environments = ["main"]

[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]
from = "draft"

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]
from = "merge"

[[checks]]
id = "desktop-build"
command = ["sh", "-c", "exit 0"]
paths = ["src/**"]
from = "merge"
"#;

/// A change to src/a.ts, which a unit test and an end-to-end spec import.
fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-stages-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["src", "test", "e2e"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    std::fs::write(dir.join(".gitignore"), "*.json\nout.txt\n").unwrap();
    std::fs::write(dir.join("src/a.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(
        dir.join("test/a.test.ts"),
        "import { a } from '../src/a';\n",
    )
    .unwrap();
    std::fs::write(dir.join("e2e/a.spec.ts"), "import { a } from '../src/a';\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("src/a.ts"), "export const a = 2;\n").unwrap();
    git(&dir, &["commit", "-qam", "change"]);
    dir
}

struct Planned {
    code: i32,
    out: String,
    plan: Value,
    outputs: String,
}

/// `ci plan --format github` as GitHub Actions would start it for `event`.
fn plan(dir: &Path, event: &str, git_ref: &str, payload: Value, extra: &[&str]) -> Planned {
    let event_path = dir.join("event.json");
    std::fs::write(&event_path, payload.to_string()).unwrap();
    let outputs = dir.join("out.txt");
    let _ = std::fs::remove_file(&outputs);
    let mut args = vec![
        "ci",
        "plan",
        "--base",
        "main",
        "--format",
        "github",
        "--out",
        "plan.json",
    ];
    args.extend_from_slice(extra);
    let (code, out) = run(command(dir, env!("CARGO_BIN_EXE_fairlead"), &args)
        .env("GITHUB_ACTIONS", "true")
        .env("GITHUB_EVENT_NAME", event)
        .env("GITHUB_REF", git_ref)
        .env("GITHUB_EVENT_PATH", &event_path)
        .env("GITHUB_OUTPUT", &outputs));
    let plan = std::fs::read_to_string(dir.join("plan.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    Planned {
        code,
        out,
        plan,
        outputs: std::fs::read_to_string(&outputs).unwrap_or_default(),
    }
}

fn ids(plan: &Value) -> Vec<String> {
    plan["invocations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn output(outputs: &str, name: &str) -> String {
    let start = format!("{name}<<");
    let mut lines = outputs.lines();
    while let Some(line) = lines.next() {
        if line.starts_with(&start) {
            return lines.next().unwrap_or("").to_string();
        }
    }
    panic!("no output {name} in {outputs}");
}

fn draft() -> Value {
    json!({"pull_request": {"number": 7, "draft": true, "labels": []}})
}

#[test]
fn a_draft_runs_only_its_stage_and_defers_the_rest() {
    let dir = repo("draft", STAGED);
    let p = plan(&dir, "pull_request", "refs/pull/7/merge", draft(), &[]);
    assert_eq!(p.code, 0, "{}", p.out);
    assert!(
        p.out.contains("stage: draft (auto: pull_request, a draft)"),
        "{}",
        p.out
    );
    assert_eq!(ids(&p.plan), ["unit"]);
    assert_eq!(p.plan["stage"], "draft");
    let deferred: Vec<(String, String, u64)> = p.plan["deferred"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["id"].as_str().unwrap().into(),
                d["from"].as_str().unwrap().into(),
                d["selected"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        deferred,
        [
            ("e2e".into(), "merge".into(), 1),
            ("desktop-build".into(), "merge".into(), 1)
        ]
    );
    assert!(
        p.out
            .contains("  e2e waits for the merge stage (1 selected)"),
        "{}",
        p.out
    );
    assert_eq!(output(&p.outputs, "stage"), "draft");
    assert_eq!(output(&p.outputs, "run_unit"), "true");
    assert_eq!(output(&p.outputs, "run_e2e"), "false");
    assert_eq!(output(&p.outputs, "run_desktop_build"), "false");
    let (code, report) = run(&mut command(
        &dir,
        env!("CARGO_BIN_EXE_fairlead"),
        &["ci", "report", "--plan", "plan.json"],
    ));
    assert_eq!(code, 0, "{report}");
    assert!(
        report.contains("Stage: draft. Waiting for a later stage: `e2e` (from merge, 1 selected), `desktop-build` (from merge, 1 selected)."),
        "{report}"
    );
}

#[test]
fn a_push_to_an_environment_is_the_merge_stage_and_runs_everything_from_it() {
    let dir = repo("merge", STAGED);
    let p = plan(&dir, "push", "refs/heads/main", json!({}), &[]);
    assert_eq!(p.code, 0, "{}", p.out);
    assert_eq!(p.plan["stage"], "merge");
    assert_eq!(ids(&p.plan), ["unit", "e2e", "desktop-build"]);
    assert!(p.plan.get("deferred").is_none(), "{}", p.plan);
    assert_eq!(output(&p.outputs, "run_e2e"), "true");
    let other = plan(&dir, "push", "refs/heads/feature/x", json!({}), &[]);
    assert_eq!(other.plan["stage"], "ready");
    assert_eq!(ids(&other.plan), ["unit"]);
}

#[test]
fn a_schedule_runs_everything_and_each_stage_has_its_own_plan_id() {
    let dir = repo("full", STAGED);
    let full = plan(&dir, "schedule", "refs/heads/main", json!({}), &[]);
    assert_eq!(full.code, 0, "{}", full.out);
    assert_eq!(full.plan["stage"], "full");
    assert_eq!(full.plan["all"], true);
    let d = plan(&dir, "pull_request", "refs/pull/7/merge", draft(), &[]);
    let r = plan(
        &dir,
        "pull_request",
        "refs/pull/7/merge",
        draft(),
        &["--stage", "ready"],
    );
    assert_eq!(r.plan["stage"], "ready");
    assert_ne!(d.plan["plan_id"], r.plan["plan_id"]);
    assert_eq!(d.plan["config_digest"], r.plan["config_digest"]);
}

#[test]
fn a_config_without_stages_plans_exactly_as_before() {
    let dir = repo("none", RUNNERS);
    let p = plan(&dir, "pull_request", "refs/pull/7/merge", draft(), &[]);
    assert_eq!(p.code, 0, "{}", p.out);
    assert!(!p.out.contains("stage:"), "{}", p.out);
    assert!(p.plan.get("stage").is_none() && p.plan.get("deferred").is_none());
    assert_eq!(ids(&p.plan), ["unit", "e2e", "desktop-build"]);
    assert_eq!(output(&p.outputs, "stage"), "");
    // `from` and `[stages]` leave the digest alone, and `--stage none` plans as `fairlead plan` does.
    let staged = repo("none-staged", STAGED);
    let s = plan(
        &staged,
        "pull_request",
        "refs/pull/7/merge",
        draft(),
        &["--stage", "none"],
    );
    assert_eq!(s.plan["config_digest"], p.plan["config_digest"]);
    assert_eq!(ids(&s.plan), ["unit", "e2e", "desktop-build"]);
    let (code, local) = run(&mut command(
        &staged,
        env!("CARGO_BIN_EXE_fairlead"),
        &["plan", "--base", "main", "--json"],
    ));
    assert_eq!(code, 0, "{local}");
    let local: Value = serde_json::from_str(&local).unwrap();
    assert_eq!(s.plan["plan_id"], local["plan_id"]);
}

#[test]
fn ci_run_only_runs_the_named_steps_and_a_deferred_one_is_not_an_error() {
    let dir = repo("only", STAGED);
    let merge = plan(&dir, "push", "refs/heads/main", json!({}), &[]);
    assert_eq!(merge.code, 0, "{}", merge.out);
    let ci_run = |args: &[&str]| {
        let mut all = vec!["ci", "run", "--plan", "plan.json"];
        all.extend_from_slice(args);
        run(&mut command(&dir, env!("CARGO_BIN_EXE_fairlead"), &all))
    };
    let (code, out) = ci_run(&["--only", "e2e"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 invocations passed"), "{out}");
    let (code, out) = ci_run(&["--except", "e2e"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("2 invocations passed"), "{out}");
    let (code, out) = ci_run(&["--only", "e2e-typo"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("no runner or check is called `e2e-typo`"),
        "{out}"
    );

    plan(&dir, "pull_request", "refs/pull/7/merge", draft(), &[]);
    let (code, out) = ci_run(&["--only", "e2e"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("e2e waits for the merge stage; this plan is draft"),
        "{out}"
    );
    assert!(out.contains("0 invocations passed"), "{out}");
}

#[test]
fn config_check_refuses_an_unknown_stage() {
    let dir = repo(
        "check",
        &STAGED.replace("from = \"merge\"", "from = \"later\""),
    );
    let (code, out) = run(&mut command(
        &dir,
        env!("CARGO_BIN_EXE_fairlead"),
        &["config", "check"],
    ));
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("checks"), "{out}");
}
