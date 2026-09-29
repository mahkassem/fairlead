//! `fairlead ci run --results` and `fairlead ci report` on a real repository:
//! the summary of what was selected and why, what ran and failed, the
//! receipt, and the pull request comment kept up to date by its marker.

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

fn git(dir: &Path, args: &[&str]) {
    let out = command(dir, "git", args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fairlead(cmd: &mut Command) -> (i32, String) {
    let out = cmd.output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

fn bin(dir: &Path, args: &[&str]) -> Command {
    command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
}

const CONFIG: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "test ! -f FAIL", "sh", "{files}"]
"#;

/// A repository with a planned change and its plan written to `plan.json`.
fn planned(name: &str, fail: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-report-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join(".gitignore"), "FAIL\n*.json\nsummary.md\n").unwrap();
    std::fs::write(dir.join("src/a.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(
        dir.join("test/a.test.ts"),
        "import { a } from '../src/a';\n",
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("src/a.ts"), "export const a = 2;\n").unwrap();
    if fail {
        std::fs::write(dir.join("FAIL"), "").unwrap();
    }
    let (code, out) = fairlead(&mut bin(
        &dir,
        &["ci", "plan", "--base", "main", "--out", "plan.json"],
    ));
    assert_eq!(code, 0, "{out}");
    dir
}

#[test]
fn the_report_says_what_was_selected_and_why_what_ran_and_how_to_rerun_a_failure() {
    let dir = planned("failed", true);
    let (code, _) = fairlead(&mut bin(
        &dir,
        &[
            "ci",
            "run",
            "--plan",
            "plan.json",
            "--results",
            "results.json",
        ],
    ));
    assert_eq!(code, 1);
    let results: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("results.json")).unwrap()).unwrap();
    assert_eq!(results["passed"], false);
    assert_eq!(results["invocations"][0]["id"], "unit");
    assert_eq!(results["invocations"][0]["passed"], false);
    std::fs::write(
        dir.join("receipt.txt"),
        "receipt for brief b-1\nchanged  1 file\n",
    )
    .unwrap();
    let summary = dir.join("summary.md");
    let (code, out) = fairlead(
        bin(
            &dir,
            &[
                "ci",
                "report",
                "--plan",
                "plan.json",
                "--results",
                "results.json",
                "--receipt",
                "receipt.txt",
            ],
        )
        .env("GITHUB_STEP_SUMMARY", &summary),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("the report is in the step summary"), "{out}");
    let md = std::fs::read_to_string(&summary).unwrap();
    assert!(
        md.starts_with("<!-- fairlead-report -->\n### Fairlead: 1 of 1 invocations failed"),
        "{md}"
    );
    assert!(
        md.contains("1 changed file, selected 1 test file and 0 checks"),
        "{md}"
    );
    assert!(md.contains("| it imports a changed file | 1 |"), "{md}");
    assert!(md.contains("| `unit` in `.` | **failed** |"), "{md}");
    assert!(
        md.contains("(cd . && sh -c test ! -f FAIL sh test/a.test.ts)"),
        "{md}"
    );
    assert!(
        md.contains("<summary>Receipt</summary>") && md.contains("receipt for brief b-1"),
        "{md}"
    );
}

#[test]
fn results_for_another_plan_are_refused() {
    let dir = planned("mismatch", false);
    fairlead(&mut bin(
        &dir,
        &[
            "ci",
            "run",
            "--plan",
            "plan.json",
            "--results",
            "results.json",
        ],
    ));
    let mut results: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("results.json")).unwrap()).unwrap();
    assert_eq!(results["passed"], true);
    results["plan_id"] = json!("pl_other");
    std::fs::write(dir.join("results.json"), results.to_string()).unwrap();
    let (code, out) = fairlead(&mut bin(
        &dir,
        &[
            "ci",
            "report",
            "--plan",
            "plan.json",
            "--results",
            "results.json",
        ],
    ));
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("the results are for plan pl_other"), "{out}");
}

/// A GitHub API stand-in on localhost: it records each request and answers
/// the comment list with whatever `comments` holds.
fn fake_api(
    comments: Arc<Mutex<Vec<Value>>>,
    seen: Arc<Mutex<Vec<(String, String, Value)>>>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            );
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
            let answer = if method == "GET" {
                Value::Array(comments.lock().unwrap().clone())
            } else {
                json!({"id": 7})
            };
            seen.lock().unwrap().push((method, path, body));
            let text = answer.to_string();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            )
            .unwrap();
        }
    });
    url
}

#[test]
fn the_comment_is_posted_once_and_then_updated_in_place() {
    let dir = planned("comment", false);
    fairlead(&mut bin(
        &dir,
        &[
            "ci",
            "run",
            "--plan",
            "plan.json",
            "--results",
            "results.json",
        ],
    ));
    std::fs::write(dir.join("event.json"), r#"{"pull_request": {"number": 5}}"#).unwrap();
    let comments = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let api = fake_api(comments.clone(), seen.clone());
    let report = |dir: &Path| {
        let mut cmd = bin(
            dir,
            &[
                "ci",
                "report",
                "--plan",
                "plan.json",
                "--results",
                "results.json",
                "--comment",
            ],
        );
        cmd.env("GITHUB_API_URL", &api)
            .env("GITHUB_TOKEN", "t")
            .env("GITHUB_REPOSITORY", "o/r")
            .env("GITHUB_EVENT_PATH", dir.join("event.json"));
        fairlead(&mut cmd)
    };
    let (code, out) = report(&dir);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("posted the report comment on #5"), "{out}");
    comments
        .lock()
        .unwrap()
        .push(json!({"id": 7, "body": "<!-- fairlead-report -->\nold"}));
    let (_, out) = report(&dir);
    assert!(out.contains("updated the report comment on #5"), "{out}");
    let seen = seen.lock().unwrap();
    let calls: Vec<(&str, &str)> = seen
        .iter()
        .map(|(m, p, _)| (m.as_str(), p.as_str()))
        .collect();
    assert_eq!(
        calls,
        [
            ("GET", "/repos/o/r/issues/5/comments?per_page=100"),
            ("POST", "/repos/o/r/issues/5/comments"),
            ("GET", "/repos/o/r/issues/5/comments?per_page=100"),
            ("PATCH", "/repos/o/r/issues/comments/7"),
        ]
    );
    let posted = seen[1].2["body"].as_str().unwrap();
    assert!(
        posted.starts_with("<!-- fairlead-report -->\n### Fairlead: passed"),
        "{posted}"
    );
}
