//! Tree reuse through the binary against a stand-in GitHub API: a ready
//! pull request run records the tree it passed, and a push of that exact
//! tree skips the ready stage's steps; one byte different, or no
//! permission to record, and they run.

use std::io::{BufRead, BufReader, Read, Write};
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

fn run(cmd: &mut Command) -> (i32, String) {
    let out = cmd.output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

const STAGED: &str = r#"
[stages]
environments = ["main"]

[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]
from = "merge"
"#;

const PR_HEAD: &str = "1111111111111111111111111111111111111111";

/// The API calls the binary makes, and what the stand-in answers.
#[derive(Default)]
struct Api {
    statuses: Vec<Value>,
    posted: Vec<(String, Value)>,
    refuse_status: bool,
}

fn fake_api(api: Arc<Mutex<Api>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let mut api = api.lock().unwrap();
            let (status, answer) = if method == "POST" && path.contains("/statuses/") {
                api.posted.push((path.clone(), body.clone()));
                if api.refuse_status {
                    (
                        403,
                        json!({"message": "Resource not accessible by integration"}),
                    )
                } else {
                    let mut status = body.clone();
                    status["state"] = json!("success");
                    api.statuses.insert(0, status);
                    (201, json!({"id": 1}))
                }
            } else if path.ends_with("/pulls") {
                (
                    200,
                    json!([{"number": 7, "merged_at": "2026-10-06T00:00:00Z", "head": {"sha": PR_HEAD}}]),
                )
            } else if path.contains(&format!("/commits/{PR_HEAD}/statuses")) {
                (200, Value::Array(api.statuses.clone()))
            } else {
                (404, json!({"message": "Not Found"}))
            };
            let text = answer.to_string();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            )
            .unwrap();
        }
    });
    url
}

/// A branch `change` one commit past `main`, touching code a unit test and a spec import.
fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-reuse-{name}-{}", std::process::id()));
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

/// A `fairlead` command as a GitHub Actions job for `event` would start it.
fn in_actions(
    dir: &Path,
    api: &str,
    event: &str,
    git_ref: &str,
    payload: Value,
    args: &[&str],
) -> (i32, String, String) {
    let event_path = dir.join("event.json");
    std::fs::write(&event_path, payload.to_string()).unwrap();
    let outputs = dir.join("out.txt");
    let _ = std::fs::remove_file(&outputs);
    let (code, out) = run(command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .env("GITHUB_ACTIONS", "true")
        .env("GITHUB_EVENT_NAME", event)
        .env("GITHUB_REF", git_ref)
        .env("GITHUB_EVENT_PATH", &event_path)
        .env("GITHUB_OUTPUT", &outputs)
        .env("GITHUB_REPOSITORY", "acme/app")
        .env("GITHUB_API_URL", api)
        .env("GITHUB_TOKEN", "test-token"));
    (
        code,
        out,
        std::fs::read_to_string(&outputs).unwrap_or_default(),
    )
}

const PLAN: [&str; 9] = [
    "ci",
    "plan",
    "--base",
    "HEAD~1",
    "--format",
    "github",
    "--out",
    "plan.json",
    "--head",
];

fn ready_run(dir: &Path, api: &str) -> String {
    let payload = json!({"pull_request": {"number": 7, "draft": false, "labels": [], "head": {"sha": PR_HEAD}}});
    let head = git(dir, &["rev-parse", "HEAD"]);
    let mut args = PLAN.to_vec();
    args.push(&head);
    let (code, out, _) = in_actions(
        dir,
        api,
        "pull_request",
        "refs/pull/7/merge",
        payload.clone(),
        &args,
    );
    assert_eq!(code, 0, "{out}");
    let (code, out, _) = in_actions(
        dir,
        api,
        "pull_request",
        "refs/pull/7/merge",
        payload,
        &["ci", "run", "--plan", "plan.json"],
    );
    assert_eq!(code, 0, "{out}");
    out
}

fn merge_plan(dir: &Path, api: &str) -> (String, String, Value) {
    let head = git(dir, &["rev-parse", "HEAD"]);
    let mut args = PLAN.to_vec();
    args.push(&head);
    let (code, out, outputs) = in_actions(
        dir,
        api,
        "push",
        "refs/heads/main",
        json!({"after": head}),
        &args,
    );
    assert_eq!(code, 0, "{out}");
    let plan =
        serde_json::from_str(&std::fs::read_to_string(dir.join("plan.json")).unwrap()).unwrap();
    (out, outputs, plan)
}

fn ids(plan: &Value) -> Vec<&str> {
    plan["invocations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect()
}

#[test]
fn a_merge_of_the_tree_the_pull_request_passed_skips_the_ready_steps() {
    let api_state = Arc::new(Mutex::new(Api::default()));
    let api = fake_api(api_state.clone());
    let dir = repo("same", STAGED);
    let out = ready_run(&dir, &api);
    assert!(
        out.contains(&format!("recorded fairlead/tree on {PR_HEAD}")),
        "{out}"
    );
    let tree = git(&dir, &["rev-parse", "HEAD^{tree}"]);
    {
        let state = api_state.lock().unwrap();
        let (path, body) = &state.posted[0];
        assert_eq!(path, &format!("/repos/acme/app/statuses/{PR_HEAD}"));
        assert_eq!(body["context"], "fairlead/tree");
        assert!(
            body["description"]
                .as_str()
                .unwrap()
                .starts_with(&format!("tree {tree} config ")),
            "{body}"
        );
    }

    git(&dir, &["checkout", "-q", "main"]);
    git(&dir, &["merge", "-q", "--ff-only", "change"]);
    let (out, outputs, plan) = merge_plan(&dir, &api);
    assert!(
        out.contains("reuse: #7 passed this tree; skipping unit"),
        "{out}"
    );
    assert_eq!(ids(&plan), ["e2e"]);
    assert_eq!(
        plan["reused"],
        json!({"pull_request": 7, "steps": ["unit"]})
    );
    assert!(outputs.contains("reused<<FAIRLEAD_EOF_0\n7\n"), "{outputs}");
    assert!(
        outputs.contains("run_unit<<FAIRLEAD_EOF_0\nfalse\n"),
        "{outputs}"
    );
}

#[test]
fn one_byte_different_runs_the_ready_steps_again() {
    let api_state = Arc::new(Mutex::new(Api::default()));
    let api = fake_api(api_state.clone());
    let dir = repo("moved", STAGED);
    ready_run(&dir, &api);
    git(&dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("src/b.ts"), "export const b = 1;\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "another merge first"]);
    git(&dir, &["merge", "-q", "--no-edit", "change"]);
    let (out, _, plan) = merge_plan(&dir, &api);
    assert!(
        out.contains("reuse: no, #7 passed another tree or config"),
        "{out}"
    );
    assert_eq!(ids(&plan), ["unit", "e2e"]);
    assert!(plan.get("reused").is_none());
}

#[test]
fn without_permission_to_record_the_run_passes_and_says_why() {
    let api_state = Arc::new(Mutex::new(Api {
        refuse_status: true,
        ..Api::default()
    }));
    let api = fake_api(api_state.clone());
    let dir = repo("refused", STAGED);
    let out = ready_run(&dir, &api);
    assert!(out.contains("couldn't record fairlead/tree, so a merge runs these steps again: 403: give the job `statuses: write`"), "{out}");
    git(&dir, &["checkout", "-q", "main"]);
    git(&dir, &["merge", "-q", "--ff-only", "change"]);
    let (out, _, plan) = merge_plan(&dir, &api);
    assert!(
        out.contains("reuse: no pull request recorded a tree for this commit"),
        "{out}"
    );
    assert_eq!(ids(&plan), ["unit", "e2e"]);
}

#[test]
fn reuse_off_or_no_stages_records_nothing() {
    let api_state = Arc::new(Mutex::new(Api::default()));
    let api = fake_api(api_state.clone());
    let off = repo(
        "off",
        &STAGED.replace(
            "environments = [\"main\"]",
            "environments = [\"main\"]\nreuse = false",
        ),
    );
    let out = ready_run(&off, &api);
    assert!(!out.contains("fairlead/tree"), "{out}");
    let plain = repo(
        "plain",
        &STAGED
            .replace("[stages]\nenvironments = [\"main\"]\n", "")
            .replace("from = \"merge\"\n", ""),
    );
    let out = ready_run(&plain, &api);
    assert!(!out.contains("fairlead/tree"), "{out}");
    assert!(api_state.lock().unwrap().posted.is_empty());
}
