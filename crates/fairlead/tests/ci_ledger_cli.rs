//! `fairlead ci ledger` against a recorded Actions API on a local port: each
//! failure's verdict, the default branch's red streaks, the cache that
//! `next` and `doctor` read without the network, the section `ci report`
//! carries, and `--rerun` on HEAD and on the merge base.

use std::io::{BufRead, BufReader, Write};
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
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn bin(dir: &Path, args: &[&str]) -> Command {
    command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
}

fn output(cmd: &mut Command) -> (i32, String) {
    let out = cmd.output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

const CONFIG: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "grep -q pass \"$1\"", "sh", "{files}"]

[[replay.failures]]
runner = "unit"
extractor = "vitest"
"#;

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-ledger-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join(".gitignore"), "*.json\n").unwrap();
    for (f, text) in [("a", "pass"), ("b", "fail"), ("c", "pass")] {
        std::fs::write(dir.join(format!("test/{f}.test.ts")), text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("test/a.test.ts"), "fail").unwrap();
    git(&dir, &["commit", "-qam", "break a"]);
    dir
}

type Answer = (u16, String);

/// Answers each GET with `route(path)`: status and raw body.
fn fake_api(route: fn(&str) -> Answer) -> String {
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
            let (status, body) = route(&path);
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    url
}

fn run(id: u64, workflow_id: u64, name: &str, event: &str, sha: &str, day: u32, conclusion: &str) -> Value {
    json!({"id": id, "workflow_id": workflow_id, "name": name, "event": event, "head_sha": sha,
           "run_attempt": 1, "created_at": format!("2026-11-{day:02}T10:00:00Z"),
           "conclusion": conclusion, "status": "completed",
           "pull_requests": if event == "pull_request" { json!([{"number": 5, "base": {"ref": "main"}}]) } else { json!([]) }})
}

fn job(id: u64, name: &str, conclusion: &str) -> Value {
    json!({"id": id, "name": name, "conclusion": conclusion,
           "steps": [{"name": "Test", "conclusion": conclusion}]})
}

const A: &str = " FAIL  test/a.test.ts > adds\nAssertionError: expected 1 to be 2\n";
const B: &str = " FAIL  test/b.test.ts > subtracts\nError: boom at test/b.test.ts:3:9\n";

/// The pull request's run 100 fails a (own) and b (red on main since run
/// 90) in `unit`, and c in `e2e` on a timeout. `nightly` is red twice, never
/// green, for a different reason each night.
fn history(path: &str) -> Answer {
    let ok = |v: Value| (200, v.to_string());
    let text = |t: &str| (200, t.to_string());
    let key = path.split('?').next().unwrap_or("");
    let query = path.split('?').nth(1).unwrap_or("");
    match key {
        "/repos/o/r/actions/runs" => {
            let sha = query.split('&').find_map(|p| p.strip_prefix("head_sha=")).unwrap_or("");
            ok(json!({"workflow_runs": [run(100, 7, "ci", "pull_request", sha, 8, "failure")]}))
        }
        "/repos/o/r" => ok(json!({"default_branch": "main"})),
        "/repos/o/r/actions/runs/100/attempts/1/jobs" => ok(json!({"jobs": [
            job(1001, "unit", "failure"), job(1002, "e2e", "failure"), job(1003, "lint", "success")]})),
        "/repos/o/r/actions/runs/90/attempts/1/jobs" => ok(json!({"jobs": [
            job(901, "unit", "failure"), job(902, "e2e", "success")]})),
        "/repos/o/r/actions/runs/80/attempts/1/jobs" => ok(json!({"jobs": [
            job(801, "unit", "success"), job(802, "e2e", "success")]})),
        "/repos/o/r/actions/runs/70/attempts/1/jobs" => ok(json!({"jobs": [job(701, "e2e", "failure")]})),
        "/repos/o/r/actions/runs/60/attempts/1/jobs" => ok(json!({"jobs": [job(601, "e2e", "failure")]})),
        "/repos/o/r/actions/workflows" => ok(json!({"workflows": [
            {"id": 7, "name": "ci", "state": "active"}, {"id": 8, "name": "nightly", "state": "active"}]})),
        "/repos/o/r/actions/workflows/7/runs" => ok(json!({"total_count": 40, "workflow_runs": [
            run(100, 7, "ci", "pull_request", "head", 8, "failure"),
            run(90, 7, "ci", "push", "b90c0ffee", 7, "failure"),
            run(80, 7, "ci", "push", "b80c0ffee", 6, "success")]})),
        "/repos/o/r/actions/workflows/8/runs" => ok(json!({"total_count": 2, "workflow_runs": [
            run(70, 8, "nightly", "schedule", "b90c0ffee", 7, "failure"),
            run(60, 8, "nightly", "schedule", "b80c0ffee", 6, "failure")]})),
        "/repos/o/r/actions/jobs/1001/logs" => text(&format!("{A}{B}")),
        "/repos/o/r/actions/jobs/1002/logs" => text(" FAIL  test/c.test.ts > waits\nError: Test timed out in 5000ms.\n"),
        "/repos/o/r/actions/jobs/901/logs" => text(&format!("{B}")),
        "/repos/o/r/actions/jobs/701/logs" => text(" FAIL  test/c.test.ts > waits\nError: y\n"),
        "/repos/o/r/actions/jobs/601/logs" => text(" FAIL  test/b.test.ts > subtracts\nError: x\n"),
        k if k.ends_with("/annotations") => ok(json!([])),
        _ => (404, "{}".into()),
    }
}

fn ledger(dir: &Path, api: &str, extra: &[&str]) -> (i32, String) {
    let head = git(dir, &["rev-parse", "HEAD"]);
    let mut args = vec!["ci", "ledger", "--repo", "o/r", "--sha", &head];
    args.extend(extra);
    output(
        bin(dir, &args)
            .env("GITHUB_TOKEN", "t")
            .env("GITHUB_API_URL", api),
    )
}

#[test]
fn each_failure_gets_a_verdict_and_the_cache_speaks_for_head_offline() {
    let dir = repo("verdicts");
    let api = fake_api(history);
    let (code, out) = ledger(&dir, &api, &["--out", "ledger.json"]);
    assert_eq!(code, 0, "{out}");
    let first = out.lines().next().unwrap();
    assert!(
        first.ends_with("1 of 3 failures inherited: red on main since b90c0ff (2026-11-07); 1 flaky; 1 own"),
        "{out}"
    );
    assert!(out.contains("  own       unit  test/a.test.ts > adds: not failing on main"), "{out}");
    assert!(out.contains("  flaky     e2e  test/c.test.ts > waits: it timed out with no assertion failing"), "{out}");
    assert!(
        out.contains("streak  nightly (schedule) red on main for 2 runs since b80c0ff (2026-11-06)"),
        "{out}"
    );
    assert!(out.contains("never green since it was added"), "{out}");
    assert!(out.contains("the cause drifted in e2e"), "{out}");
    assert!(
        out.contains("propose [[replay.quarantine]] path = \"test/c.test.ts\", job = \"^e2e$\""),
        "{out}"
    );
    let head = git(&dir, &["rev-parse", "HEAD"]);
    assert!(dir.join(format!(".git/fairlead/ledger/{head}.json")).is_file());
    // With the API gone, `next` and `doctor` still read the cache.
    let (_, next) = output(&mut bin(&dir, &["next"]));
    assert!(next.contains("ci: 1 of 3 failures inherited"), "{next}");
    assert!(next.contains("min old"), "{next}");
    let (_, doctor) = output(&mut bin(&dir, &["doctor"]));
    assert!(doctor.contains("streaks: 2 red"), "{doctor}");
    assert!(doctor.contains("nightly (schedule) red on main for 2 runs"), "{doctor}");
    // A new commit leaves the cache behind: it no longer speaks for HEAD.
    std::fs::write(dir.join("test/c.test.ts"), "pass again").unwrap();
    git(&dir, &["commit", "-qam", "touch c"]);
    let (_, next) = output(&mut bin(&dir, &["next"]));
    assert!(!next.contains("ci:"), "{next}");
}

#[test]
fn the_report_carries_the_verdicts_once() {
    let dir = repo("report");
    let api = fake_api(history);
    let (code, out) = ledger(&dir, &api, &["--out", "ledger.json", "--streaks", "0"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = output(&mut bin(&dir, &["ci", "plan", "--base", "main", "--out", "plan.json"]));
    assert_eq!(code, 0, "{out}");
    let (code, report) = output(&mut bin(
        &dir,
        &["ci", "report", "--plan", "plan.json", "--ledger", "ledger.json"],
    ));
    assert_eq!(code, 0, "{report}");
    assert_eq!(report.matches("#### Is this red mine?").count(), 1, "{report}");
    assert!(report.contains("| inherited | `unit` | `test/b.test.ts > subtracts` | red on main since b90c0ff (2026-11-07) |"), "{report}");
    assert!(!report.contains("Red on the default branch"), "{report}");
}

#[test]
fn without_a_token_the_ledger_says_how_to_get_one() {
    let dir = repo("token");
    // No `gh` on this PATH either.
    let (code, out) = output(
        bin(&dir, &["ci", "ledger", "--repo", "o/r"]).env("PATH", dir.join("nothing")),
    );
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("no GITHUB_TOKEN or GH_TOKEN"), "{out}");
}

#[test]
fn a_rerun_tells_flaky_from_inherited_from_a_regression_on_the_merge_base() {
    let dir = repo("rerun");
    let failure = |file: &str| {
        json!({"workflow": "ci", "run_id": 1, "attempt": 1, "job": "unit", "test": file, "file": file,
               "message": "x", "fingerprint": "0", "verdict": "own", "why": "not failing on main"})
    };
    let input = json!({"version": 1, "repo": "o/r", "made_at": "2026-11-08T10:00:00Z",
        "head_sha": git(&dir, &["rev-parse", "HEAD"]), "base_branch": "main", "runs": [],
        "failures": [failure("test/a.test.ts"), failure("test/b.test.ts"), failure("test/c.test.ts"),
                     failure("test/gone.test.ts")]});
    std::fs::write(dir.join("in.json"), input.to_string()).unwrap();
    let (code, out) = output(&mut bin(
        &dir,
        &["ci", "ledger", "--input", "in.json", "--rerun", "--json"],
    ));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("test/a.test.ts: failed again on HEAD and passed on the merge base"), "{out}");
    assert!(out.contains("test/b.test.ts: failed again on HEAD and on the merge base"), "{out}");
    assert!(out.contains("test/c.test.ts: passed on a rerun of HEAD"), "{out}");
    assert!(out.contains("test/gone.test.ts: no runner runs it here"), "{out}");
    let json_start = out.find("{\n").unwrap();
    let json_end = out.rfind('}').unwrap();
    let made: Value = serde_json::from_str(&out[json_start..=json_end]).unwrap();
    let verdicts: Vec<&str> = made["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["verdict"].as_str().unwrap())
        .collect();
    assert_eq!(verdicts, ["own", "inherited", "flaky", "own"]);
    assert_eq!(made["proposals"][0]["path"], "test/c.test.ts");
    assert!(git(&dir, &["worktree", "list"]).lines().count() == 1, "the temporary worktree is gone");
    // A zero time cap reruns nothing.
    let (_, out) = output(&mut bin(
        &dir,
        &["ci", "ledger", "--input", "in.json", "--rerun", "--rerun-seconds", "0"],
    ));
    assert!(out.contains("test/a.test.ts: not rerun: the time cap ran out"), "{out}");
}
