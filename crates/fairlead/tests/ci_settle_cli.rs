//! `fairlead ci settle` on a real repository: a run that ran everything
//! settles, a step no job ran does not and is named with its test files, a
//! re-run's outcome replaces the run it repeats, and a plan that is another
//! commit's, or that leaves part of the change out, is refused.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
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

fn git(dir: &Path, args: &[&str]) -> String {
    let out = command(dir, "git", args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fairlead_with(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = command(dir, env!("CARGO_BIN_EXE_fairlead"), args);
    for (name, value) in env {
        cmd.env(name, value);
    }
    let out = cmd.output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    fairlead_with(dir, args, &[])
}

// A runner fails while a file named `FAIL-<id>` is in the repository.
const CONFIG: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "test ! -f FAIL-unit", "sh", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["sh", "-c", "test ! -f FAIL-e2e", "sh", "{files}"]

[[checks]]
id = "lint"
command = ["sh", "-c", "test ! -f FAIL-lint"]
paths = ["src/**"]
"#;

/// A repository on a branch whose one commit reaches two unit tests, one
/// end-to-end spec and the lint check.
fn repo_with(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-settle-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["src", "test", "e2e"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    std::fs::write(dir.join(".gitignore"), "*.json\nFAIL-*\nout/\n").unwrap();
    std::fs::write(dir.join("src/x.ts"), "export const x = 1;\n").unwrap();
    for file in ["test/a.test.ts", "test/b.test.ts", "e2e/flow.spec.ts"] {
        std::fs::write(dir.join(file), "import { x } from '../src/x';\n").unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("src/x.ts"), "export const x = 2;\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "change"]);
    dir
}

fn repo(name: &str) -> PathBuf {
    repo_with(name, CONFIG)
}

fn plan(dir: &Path, extra: &[&str]) {
    let mut args = vec!["ci", "plan", "--base", "main", "--out", "plan.json"];
    args.extend(extra);
    let (code, text) = fairlead(dir, &args);
    assert_eq!(code, 0, "{text}");
}

fn run(dir: &Path, results: &str, extra: &[&str], attempt: &str) -> i32 {
    let mut args = vec!["ci", "run", "--plan", "plan.json", "--results", results];
    args.extend(extra);
    fairlead_with(dir, &args, &[("GITHUB_RUN_ATTEMPT", attempt)]).0
}

fn settle(dir: &Path, results: &[&str]) -> (i32, String) {
    let mut args = vec!["ci", "settle", "--plan", "plan.json"];
    for r in results {
        args.extend(["--results", r]);
    }
    fairlead(dir, &args)
}

#[test]
fn a_run_that_ran_every_step_green_settles() {
    let dir = repo("green");
    plan(&dir, &[]);
    assert_eq!(run(&dir, "all.json", &[], "1"), 0);
    let (code, text) = settle(&dir, &["all.json"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("settled: 3 test files in 3 steps"), "{text}");
    assert!(
        text.contains("unit: 2 test files, proven by results"),
        "{text}"
    );
    assert!(text.contains("lint: proven by results"), "{text}");
}

#[test]
fn a_step_no_job_ran_is_named_with_its_test_files() {
    let dir = repo("skipped");
    plan(&dir, &[]);
    assert_eq!(
        run(
            &dir,
            "unit.json",
            &["--only", "unit", "--only", "lint"],
            "1"
        ),
        0
    );
    let (code, text) = fairlead(
        &dir,
        &[
            "ci",
            "settle",
            "--plan",
            "plan.json",
            "--results",
            "unit.json",
            "--out",
            "debts.json",
        ],
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("not settled: 1 step"), "{text}");
    assert!(text.contains("e2e: no job ran it; 1 test file"), "{text}");
    assert!(text.contains("e2e/flow.spec.ts"), "{text}");
    assert!(text.contains("fairlead guide report"), "{text}");
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("debts.json")).unwrap()).unwrap();
    assert_eq!(record["unsettled"][0]["id"], "e2e");
    assert_eq!(record["unsettled"][0]["files"], json!(["e2e/flow.spec.ts"]));
    assert_eq!(record["settled"].as_array().unwrap().len(), 2);
    // The checkout keeps the record too, named for the commit.
    let head = git(&dir, &["rev-parse", "HEAD"]);
    assert!(dir
        .join(".git/fairlead/debts")
        .join(format!("{head}.json"))
        .is_file());
}

#[test]
fn no_results_at_all_settles_nothing() {
    let dir = repo("none");
    plan(&dir, &[]);
    let (code, text) = settle(&dir, &[]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("not settled: 3 steps"), "{text}");
}

#[test]
fn a_rerun_replaces_the_run_it_repeats() {
    let dir = repo("rerun");
    plan(&dir, &[]);
    std::fs::write(dir.join("FAIL-e2e"), "").unwrap();
    assert_eq!(run(&dir, "first.json", &[], "1"), 1);
    let (code, text) = settle(&dir, &["first.json"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("e2e: it failed; 1 test file"), "{text}");
    // The failed job runs again and passes: the newer attempt is the one that counts.
    std::fs::remove_file(dir.join("FAIL-e2e")).unwrap();
    assert_eq!(run(&dir, "second.json", &["--only", "e2e"], "2"), 0);
    let (code, text) = settle(&dir, &["first.json", "second.json"]);
    assert_eq!(code, 0, "{text}");
    // And a pass does not outlive a newer failure of the same step.
    std::fs::write(dir.join("FAIL-unit"), "").unwrap();
    assert_eq!(run(&dir, "third.json", &["--only", "unit"], "3"), 1);
    let (code, text) = settle(&dir, &["first.json", "second.json", "third.json"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("unit: it failed; 2 test files"), "{text}");
}

#[test]
fn every_shard_has_to_report_and_a_folder_of_results_is_read() {
    let dir = repo("shards");
    plan(&dir, &["--shards", "2"]);
    std::fs::create_dir_all(dir.join("out/one")).unwrap();
    std::fs::create_dir_all(dir.join("out/two")).unwrap();
    assert_eq!(run(&dir, "out/one/r.json", &["--shard", "1/2"], "1"), 0);
    let (code, text) = settle(&dir, &["out"]);
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("unit (shard 2): no job ran it; 1 test file"),
        "{text}"
    );
    assert_eq!(run(&dir, "out/two/r.json", &["--shard", "2/2"], "1"), 0);
    // The plan file sits in the folder too, and is not taken for results.
    std::fs::copy(dir.join("plan.json"), dir.join("out/plan.json")).unwrap();
    let (code, text) = settle(&dir, &["out"]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("unit: 2 test files, proven by results"),
        "{text}"
    );
}

#[test]
fn results_for_another_plan_count_for_nothing() {
    let dir = repo("foreign");
    plan(&dir, &[]);
    assert_eq!(run(&dir, "all.json", &[], "1"), 0);
    let mut results: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("all.json")).unwrap()).unwrap();
    results["plan_id"] = json!("sha256:another");
    std::fs::write(dir.join("other.json"), results.to_string()).unwrap();
    let (code, text) = settle(&dir, &["other.json"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("unit: no job ran it"), "{text}");
}

#[test]
fn a_plan_for_another_commit_or_for_uncommitted_changes_is_refused() {
    let dir = repo("stale");
    plan(&dir, &[]);
    assert_eq!(run(&dir, "all.json", &[], "1"), 0);
    std::fs::write(dir.join("src/x.ts"), "export const x = 3;\n").unwrap();
    plan(&dir, &[]);
    let (code, text) = settle(&dir, &["all.json"]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("uncommitted changes"), "{text}");
    git(&dir, &["checkout", "-q", "--", "src/x.ts"]);
    plan(&dir, &[]);
    std::fs::write(dir.join("src/x.ts"), "export const x = 4;\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "later"]);
    let (code, text) = settle(&dir, &["all.json"]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("the plan is for commit"), "{text}");
}

#[test]
fn a_cancelled_run_is_never_settled() {
    let dir = repo("cancelled");
    plan(&dir, &[]);
    assert_eq!(run(&dir, "all.json", &[], "1"), 0);
    let needs = r#"{"plan":{"result":"success"},"checks":{"result":"cancelled"}}"#;
    let (code, text) = fairlead(
        &dir,
        &[
            "ci",
            "settle",
            "--plan",
            "plan.json",
            "--results",
            "all.json",
            "--needs",
            needs,
        ],
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("cancelled before checks finished"), "{text}");
}

/// A pull request event whose base is `base`, written where GitHub would.
fn event(dir: &Path, base: &str, draft: bool) -> String {
    let path = dir.join("event.json");
    let payload = json!({"pull_request": {"draft": draft, "base": {"sha": base}, "labels": []}});
    std::fs::write(&path, payload.to_string()).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_plan_that_starts_after_the_pull_requests_base_is_refused() {
    let dir = repo("late-base");
    let base = git(&dir, &["rev-parse", "main"]);
    std::fs::write(
        dir.join("test/a.test.ts"),
        "import { x } from '../src/x'; // again\n",
    )
    .unwrap();
    git(&dir, &["commit", "-q", "-am", "second"]);
    // Planned from the commit before HEAD: the first commit's change is left out.
    let (code, text) = fairlead(
        &dir,
        &["ci", "plan", "--base", "HEAD~1", "--out", "plan.json"],
    );
    assert_eq!(code, 0, "{text}");
    let path = event(&dir, &base, false);
    let env = [
        ("GITHUB_EVENT_NAME", "pull_request"),
        ("GITHUB_EVENT_PATH", path.as_str()),
    ];
    let (code, text) = fairlead_with(&dir, &["ci", "settle", "--plan", "plan.json"], &env);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("leaves part of the change out"), "{text}");
    // Planned from the pull request's base, the same event is judged.
    plan(&dir, &[]);
    assert_eq!(run(&dir, "all.json", &[], "1"), 0);
    let (code, text) = fairlead_with(
        &dir,
        &[
            "ci",
            "settle",
            "--plan",
            "plan.json",
            "--results",
            "all.json",
        ],
        &env,
    );
    assert_eq!(code, 0, "{text}");
}

const STAGED: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "true", "sh", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["sh", "-c", "true", "sh", "{files}"]
from = "merge"
"#;

#[test]
fn a_later_stages_steps_wait_and_an_earlier_stages_plan_is_refused() {
    let dir = repo_with("staged", STAGED);
    let base = git(&dir, &["rev-parse", "main"]);
    let path = event(&dir, &base, false);
    let env = [
        ("GITHUB_EVENT_NAME", "pull_request"),
        ("GITHUB_EVENT_PATH", path.as_str()),
    ];
    plan(&dir, &["--stage", "ready"]);
    assert_eq!(run(&dir, "ready.json", &[], "1"), 0);
    let (code, text) = fairlead_with(
        &dir,
        &[
            "ci",
            "settle",
            "--plan",
            "plan.json",
            "--results",
            "ready.json",
        ],
        &env,
    );
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("waiting, not due at this stage"), "{text}");
    assert!(
        text.contains("e2e: 1 test file, due after the merge, on the push run"),
        "{text}"
    );
    // On a draft, what the ready stage runs waits for the pull request to be ready.
    let draft = event(&dir, &base, true);
    let on_draft = [
        ("GITHUB_EVENT_NAME", "pull_request"),
        ("GITHUB_EVENT_PATH", draft.as_str()),
    ];
    plan(&dir, &["--stage", "draft"]);
    let (code, text) = fairlead_with(&dir, &["ci", "settle", "--plan", "plan.json"], &on_draft);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("unit: 2 test files, due when the pull request is marked ready"),
        "{text}"
    );
    // A draft's plan on a pull request that is ready leaves the ready stage out.
    let path = event(&dir, &base, false);
    let env = [
        ("GITHUB_EVENT_NAME", "pull_request"),
        ("GITHUB_EVENT_PATH", path.as_str()),
    ];
    let (code, text) = fairlead_with(&dir, &["ci", "settle", "--plan", "plan.json"], &env);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains("the plan is for the draft stage and this run is the ready stage"),
        "{text}"
    );
}

/// A GitHub API that answers each path with what `route` gives for it.
fn fake_api(route: fn(&str) -> Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
            }
            let text = route(&path).to_string();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 200 X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            )
            .unwrap();
        }
    });
    url
}

/// The environment of run 7 of a workflow in a GitHub Actions job.
fn in_actions(api: &str) -> Vec<(&'static str, String)> {
    vec![
        ("GITHUB_REPOSITORY", "o/r".to_string()),
        ("GITHUB_RUN_ID", "7".to_string()),
        ("GITHUB_TOKEN", "t".to_string()),
        ("GITHUB_API_URL", api.to_string()),
    ]
}

fn settle_in(dir: &Path, results: &[&str], env: &[(&'static str, String)]) -> (i32, String) {
    let mut args = vec!["ci", "settle", "--plan", "plan.json"];
    for r in results {
        args.extend(["--results", r]);
    }
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    fairlead_with(dir, &args, &env)
}

fn attested_config() -> String {
    format!("{CONFIG}\n[ci.attested]\ne2e = [\"browser\", \"browser-smoke\"]\n")
}

#[test]
fn a_step_is_attested_only_when_every_job_named_for_it_succeeded_in_this_run() {
    let dir = repo_with("attested", &attested_config());
    plan(&dir, &[]);
    assert_eq!(
        run(
            &dir,
            "rest.json",
            &["--only", "unit", "--only", "lint"],
            "1"
        ),
        0
    );
    // A matrix's legs are all the job; both named jobs are green.
    let green = fake_api(|_| {
        json!({"jobs": [
            {"name": "browser (1/2)", "conclusion": "success"},
            {"name": "browser (2/2)", "conclusion": "success"},
            {"name": "browser-smoke", "conclusion": "success"},
        ]})
    });
    let (code, text) = settle_in(&dir, &["rest.json"], &in_actions(&green));
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains(
            "e2e: 1 test file, attested by jobs browser, browser-smoke, not proven by results"
        ),
        "{text}"
    );
    assert!(
        text.contains("unit: 2 test files, proven by results"),
        "{text}"
    );
    // One of the two skipped: half a proof is none.
    let skipped = fake_api(|_| {
        json!({"jobs": [
            {"name": "browser", "conclusion": "success"},
            {"name": "browser-smoke", "conclusion": "skipped"},
        ]})
    });
    let (code, text) = settle_in(&dir, &["rest.json"], &in_actions(&skipped));
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains(
            "e2e: no job ran it, and job `browser` is success, job `browser-smoke` is skipped"
        ),
        "{text}"
    );
    // Outside a run there is nothing to read, and nothing is taken on trust.
    let (code, text) = settle(&dir, &["rest.json"]);
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("job `browser` is unread: GITHUB_REPOSITORY isn't set"),
        "{text}"
    );
}

#[test]
fn results_outrank_an_attestation_and_a_wrong_step_name_is_a_config_problem() {
    let dir = repo_with("attested-results", &attested_config());
    plan(&dir, &[]);
    assert_eq!(run(&dir, "all.json", &[], "1"), 0);
    let red = fake_api(|_| json!({"jobs": [{"name": "browser", "conclusion": "failure"}]}));
    let (code, text) = settle_in(&dir, &["all.json"], &in_actions(&red));
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("e2e: 1 test file, proven by results"),
        "{text}"
    );
    std::fs::write(
        dir.join("fairlead.toml"),
        format!("{CONFIG}\n[ci.attested]\nbrowser = [\"browser\"]\n"),
    )
    .unwrap();
    let (code, text) = fairlead(&dir, &["config", "check"]);
    assert_ne!(code, 0, "{text}");
    assert!(
        text.contains("no runner or check has id `browser`"),
        "{text}"
    );
}

const NIGHTLY: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "true", "sh", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["sh", "-c", "true", "sh", "{files}"]
from = "full"
"#;

#[test]
fn what_waits_for_the_nightly_run_comes_with_how_the_last_one_ended() {
    let dir = repo_with("nightly", NIGHTLY);
    plan(&dir, &["--stage", "ready"]);
    assert_eq!(run(&dir, "ready.json", &[], "1"), 0);
    let api = fake_api(|path| match path {
        "/repos/o/r/actions/runs/7" => json!({"workflow_id": 3}),
        p if p.starts_with("/repos/o/r/actions/workflows/3/runs?event=schedule") => {
            json!({"workflow_runs": [
                {"status": "in_progress", "conclusion": null, "created_at": "2026-03-04T02:00:00Z", "head_sha": "cccccccccc"},
                {"status": "completed", "conclusion": "failure", "created_at": "2026-03-03T02:00:00Z", "head_sha": "bbbbbbbbbb"},
                {"status": "completed", "conclusion": "success", "created_at": "2026-03-02T02:00:00Z", "head_sha": "aaaaaaaaaa"},
            ]})
        }
        _ => json!({}),
    });
    let (code, text) = settle_in(&dir, &["ready.json"], &in_actions(&api));
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("e2e: 1 test file, due at the nightly run"),
        "{text}"
    );
    assert!(
        text.contains("the last nightly run, on 2026-03-03 at bbbbbbb, ended failure: what waits for the nightly run is not collected until one is green"),
        "{text}"
    );
}
