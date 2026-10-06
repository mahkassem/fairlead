//! `ci plan --since-green JOB` against a recorded Actions API served on a
//! local port: the base is the newest earlier push run where the job
//! passed, and any doubt about it plans everything with a warning.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

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
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const CONFIG: &str = r#"
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
"#;

/// Four commits on main: the base, then a change to c, to b and to a.
/// Each source file has a unit test and an end-to-end spec.
fn repo(name: &str) -> (PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("fairlead-green-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["src", "test", "e2e"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join(".gitignore"), "*.json\n").unwrap();
    for f in ["a", "b", "c"] {
        std::fs::write(dir.join(format!("src/{f}.ts")), "export const x = 1;\n").unwrap();
        let import = format!("import {{ x }} from '../src/{f}';\n");
        std::fs::write(dir.join(format!("test/{f}.test.ts")), &import).unwrap();
        std::fs::write(dir.join(format!("e2e/{f}.spec.ts")), &import).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    let mut shas = vec![git(&dir, &["rev-parse", "HEAD"])];
    for f in ["c", "b", "a"] {
        std::fs::write(dir.join(format!("src/{f}.ts")), "export const x = 2;\n").unwrap();
        git(&dir, &["commit", "-qam", f]);
        shas.push(git(&dir, &["rev-parse", "HEAD"]));
    }
    (dir, shas)
}

type Route = Box<dyn Fn(&str) -> (u16, String, Value) + Send>;

/// Answers each GET with `route(path)`: status, extra headers and JSON body.
fn fake_api(route: Route, seen: Arc<Mutex<Vec<String>>>) -> String {
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
            let (status, headers, body) = route(&path);
            seen.lock().unwrap().push(path);
            let text = body.to_string();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            )
            .unwrap();
        }
    });
    url
}

fn run(id: u64, number: u64, event: &str, sha: &str) -> Value {
    json!({"id": id, "run_number": number, "event": event, "head_branch": "main",
           "head_sha": sha, "html_url": format!("https://example.com/runs/{id}")})
}

/// The recorded history: the current run is 50 (#5) at HEAD. Newest first, a
/// later run at HEAD, the current one, a pull request run and a failed push
/// run at the change to b, a green push at the change to c, a green push at the base.
fn history(shas: &[String]) -> Route {
    let shas = shas.to_vec();
    Box::new(move |path: &str| {
        let ok = |body| (200, String::new(), body);
        if path == "/repos/o/r/actions/runs/50" {
            return ok(json!({"id": 50, "workflow_id": 7, "head_branch": "main", "run_number": 5}));
        }
        if path.starts_with("/repos/o/r/actions/workflows/7/runs?branch=main&") {
            return ok(json!({"workflow_runs": [
                run(52, 6, "push", &shas[3]),
                run(50, 5, "push", &shas[3]),
                run(49, 4, "pull_request", &shas[2]),
                run(48, 3, "push", &shas[2]),
                run(47, 2, "push", &shas[1]),
                run(46, 1, "push", &shas[0]),
            ]}));
        }
        let e2e = match path.split('/').nth(6) {
            Some("48") => "failure",
            Some("50") => "in_progress",
            _ => "success",
        };
        ok(json!({"jobs": [
            {"name": "checks", "conclusion": "success"},
            {"name": "e2e", "conclusion": e2e}
        ]}))
    })
}

struct Planned {
    code: i32,
    out: String,
    plan: Value,
}

fn plan(dir: &Path, api: &str, token: bool, event: &str, git_ref: &str) -> Planned {
    std::fs::write(dir.join("event.json"), "{}").unwrap();
    let args = ["ci", "plan", "--out", "plan.json", "--since-green", "e2e"];
    let mut cmd = command(dir, env!("CARGO_BIN_EXE_fairlead"), &args);
    cmd.env("GITHUB_ACTIONS", "true")
        .env("GITHUB_EVENT_NAME", event)
        .env("GITHUB_REF", git_ref)
        .env("GITHUB_EVENT_PATH", dir.join("event.json"))
        .env("GITHUB_REPOSITORY", "o/r")
        .env("GITHUB_RUN_ID", "50")
        .env("GITHUB_API_URL", api);
    if token {
        cmd.env("GITHUB_TOKEN", "t");
    }
    let out = cmd.output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let plan = std::fs::read_to_string(dir.join("plan.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    Planned {
        code: out.status.code().unwrap_or(-1),
        out: text,
        plan,
    }
}

fn tests_of(plan: &Value) -> Vec<String> {
    let mut paths: Vec<String> = plan["tests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["path"].as_str().map(str::to_string))
        .collect();
    paths.sort();
    paths
}

fn codes(plan: &Value) -> Vec<String> {
    plan["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["code"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn the_newest_earlier_green_push_run_is_the_base_and_pull_request_and_failed_runs_are_skipped() {
    let (dir, shas) = repo("found");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let api = fake_api(history(&shas), seen.clone());
    let p = plan(&dir, &api, true, "push", "refs/heads/main");
    assert_eq!(p.code, 0, "{}", p.out);
    assert_eq!(p.plan["base"], shas[1].as_str(), "{}", p.out);
    assert!(
        p.out.contains(&format!(
            "since-green: base {} from run 47 (#2)",
            &shas[1][..12]
        )),
        "{}",
        p.out
    );
    assert!(p.out.contains("..HEAD"), "{}", p.out);
    assert_eq!(p.plan["all"], false);
    assert_eq!(
        tests_of(&p.plan),
        [
            "e2e/a.spec.ts",
            "e2e/b.spec.ts",
            "test/a.test.ts",
            "test/b.test.ts"
        ]
    );
    assert!(codes(&p.plan).is_empty(), "{:?}", p.plan["warnings"]);
    let seen = seen.lock().unwrap();
    for skipped in ["52", "50", "49"] {
        let jobs = format!("/repos/o/r/actions/runs/{skipped}/jobs");
        assert!(!seen.iter().any(|s| s.starts_with(&jobs)), "{seen:?}");
    }
}

#[test]
fn no_token_plans_everything_with_a_warning() {
    let (dir, shas) = repo("token");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let api = fake_api(history(&shas), seen.clone());
    let p = plan(&dir, &api, false, "push", "refs/heads/main");
    assert_eq!(p.code, 0, "{}", p.out);
    assert_eq!(p.plan["all"], true);
    assert_eq!(codes(&p.plan), ["since-green-unavailable"]);
    assert!(
        p.out
            .contains("since-green: planning everything: no GITHUB_TOKEN"),
        "{}",
        p.out
    );
    assert_eq!(tests_of(&p.plan).len(), 6);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_rate_limited_api_plans_everything_with_a_warning() {
    let (dir, _) = repo("limited");
    let refuse: Route = Box::new(|_: &str| {
        (
            403,
            "x-ratelimit-remaining: 0\r\n".to_string(),
            json!({"message": "API rate limit exceeded"}),
        )
    });
    let api = fake_api(refuse, Arc::new(Mutex::new(Vec::new())));
    let p = plan(&dir, &api, true, "push", "refs/heads/main");
    assert_eq!(p.code, 0, "{}", p.out);
    assert_eq!(p.plan["all"], true);
    assert_eq!(codes(&p.plan), ["since-green-unavailable"]);
    assert!(p.out.contains("403, rate-limited"), "{}", p.out);
}

#[test]
fn a_base_missing_from_the_clone_plans_everything_and_says_to_fetch_history() {
    let (dir, mut shas) = repo("missing");
    shas[1] = "0123456789abcdef0123456789abcdef01234567".into();
    let api = fake_api(history(&shas), Arc::new(Mutex::new(Vec::new())));
    let p = plan(&dir, &api, true, "push", "refs/heads/main");
    assert_eq!(p.code, 0, "{}", p.out);
    assert_eq!(p.plan["all"], true);
    assert_eq!(codes(&p.plan), ["since-green-not-in-history"]);
    assert!(p.out.contains("fetch-depth: 0"), "{}", p.out);
}

#[test]
fn a_base_off_this_branchs_history_plans_everything() {
    let (dir, mut shas) = repo("rewritten");
    git(&dir, &["checkout", "-q", "-b", "side", &shas[0]]);
    std::fs::write(dir.join("src/c.ts"), "export const x = 3;\n").unwrap();
    git(&dir, &["commit", "-qam", "side"]);
    shas[1] = git(&dir, &["rev-parse", "HEAD"]);
    git(&dir, &["checkout", "-q", "main"]);
    let api = fake_api(history(&shas), Arc::new(Mutex::new(Vec::new())));
    let p = plan(&dir, &api, true, "push", "refs/heads/main");
    assert_eq!(p.plan["all"], true, "{}", p.out);
    assert_eq!(codes(&p.plan), ["since-green-not-in-history"]);
    assert!(p.out.contains("isn't an ancestor of HEAD"), "{}", p.out);
}

#[test]
fn no_green_run_plans_everything_with_a_warning() {
    let (dir, _) = repo("none");
    let none: Route = Box::new(|path: &str| {
        let body = if path.ends_with("/runs/50") {
            json!({"id": 50, "workflow_id": 7, "head_branch": "main", "run_number": 5})
        } else {
            json!({"workflow_runs": []})
        };
        (200, String::new(), body)
    });
    let api = fake_api(none, Arc::new(Mutex::new(Vec::new())));
    let p = plan(&dir, &api, true, "push", "refs/heads/main");
    assert_eq!(p.plan["all"], true, "{}", p.out);
    assert_eq!(codes(&p.plan), ["since-green-not-found"]);
}

#[test]
fn before_the_merge_stage_since_green_is_ignored_with_a_note() {
    let (dir, shas) = repo("ready");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let api = fake_api(history(&shas), seen.clone());
    let p = plan(&dir, &api, true, "push", "refs/heads/feature");
    assert_eq!(p.code, 0, "{}", p.out);
    assert!(
        p.out
            .contains("since-green: ignored at the ready stage; it applies at the merge stage"),
        "{}",
        p.out
    );
    assert!(codes(&p.plan).is_empty());
    assert!(seen.lock().unwrap().is_empty());
}
