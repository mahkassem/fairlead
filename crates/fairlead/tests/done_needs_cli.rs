//! `[[needs]]` in `fairlead done`: what an unmet need covers isn't run
//! here, a need with `proved_in` lets the gate pass, and one without it
//! keeps the gate open with what to do.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run(dir: &Path, program: &str, args: &[&str]) -> (i32, String) {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    let out = cmd
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

fn done(dir: &Path, args: &[&str]) -> (i32, String) {
    let mut all = vec!["done", "--base", "main"];
    all.extend_from_slice(args);
    run(dir, env!("CARGO_BIN_EXE_fairlead"), &all)
}

/// The Stop hook as Claude Code runs it: its exit code and stderr.
fn stop(dir: &Path) -> (i32, String) {
    use std::io::Write as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .args(["guard", "stop"])
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // A transcript with an edit, so the hook knows the session changed files.
    let transcript = dir.join(".git/transcript.jsonl");
    let edit = r#"{"message":{"content":[{"type":"tool_use","name":"Edit"}]}}"#;
    std::fs::write(&transcript, edit).unwrap();
    let input = serde_json::json!({
        "session_id": "s", "cwd": dir, "hook_event_name": "Stop", "stop_hook_active": false,
        "transcript_path": transcript
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    (out.status.code().unwrap_or(-1), err)
}

/// A port nothing listens on.
fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn config(port: u16, need: &str) -> String {
    format!(
        r#"[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{{files}}"]

[[tests.runners]]
id = "api"
match = ["api/**"]
command = ["sh", "-c", "for f; do echo ran $f; done", "sh", "{{files}}"]

[[checks]]
id = "db-lint"
command = ["sh", "-c", "exit 1"]

[done]
always = ["db-lint"]
guard = false

[[needs]]
id = "postgres"
tcp = ["127.0.0.1:{port}"]
timeout_ms = 200
{need}
"#
    )
}

const PROVED: &str = "tests = [\"api/**\"]\nchecks = [\"db-lint\"]\nproved_in = \"CI (api job)\"\nhint = \"Start Postgres on 5432\"";

/// Two API tests and a unit test, all reaching `src/a.ts`, which the change edits.
fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-needs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["src", "test", "api"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    run(&dir, "git", &["init", "-q", "-b", "main"]);
    let files = [
        ("fairlead.toml", config),
        ("src/a.ts", "export const a = 1;\n"),
        ("test/a.test.ts", "import { a } from '../src/a';\n"),
        ("api/one.test.ts", "import { a } from '../src/a';\n"),
        ("api/two.test.ts", "import { a } from '../src/a';\n"),
    ];
    for (path, text) in files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    run(&dir, "git", &["add", "-A"]);
    run(&dir, "git", &["commit", "-q", "-m", "base"]);
    std::fs::write(dir.join("src/a.ts"), "export const a = 2;\n").unwrap();
    dir
}

#[test]
fn an_unmet_need_proved_elsewhere_leaves_its_tests_and_checks_out_and_passes() {
    let port = closed_port();
    let dir = repo("proved", &config(port, PROVED));
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 0, "{out}");
    // Windows retries a refused connection until the timeout, so it reads as timed out there.
    let line = |why: &str| {
        format!("done: api: 2 files not run here: need \"postgres\" not met (127.0.0.1:{port} {why}); proved in CI (api job). To run them here: Start Postgres on 5432")
    };
    assert!(
        out.contains(&line("refused")) || out.contains(&line("timed out")),
        "{out}"
    );
    assert!(
        out.contains("done: db-lint: not run here: need \"postgres\""),
        "{out}"
    );
    assert!(!out.contains("ran api/"), "{out}");
    assert!(out.contains("done: passed unit"), "{out}");
    assert!(
        out.contains("done: passed, 1 steps; your change clean; environment: 2 skipped (postgres: proved in CI (api job))"),
        "{out}"
    );
    let (code, out) = done(&dir, &["--check"]);
    assert_eq!((code, out.trim()), (0, "done: passed for this tree"));
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    let last: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(
        last["unmet"],
        serde_json::json!([
            {"need": "postgres", "step": "api", "files": 2, "proved": true},
            {"need": "postgres", "step": "db-lint", "files": 0, "proved": true}
        ])
    );
    let (code, err) = stop(&dir);
    assert_eq!(code, 2, "the Stop hook says once what was left: {err}");
    assert!(
        err.contains("2 step(s) weren't run here because need \"postgres\" isn't met on this machine. Say so in the pull request"),
        "{err}"
    );
    assert_eq!(stop(&dir).0, 0, "and then lets the agent stop");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_need_covering_some_named_tests_leaves_only_those_out_and_a_met_need_runs_all() {
    let port = closed_port();
    let some = "tests = [\"api/two.test.ts\"]\nproved_in = \"CI\"";
    let dir = repo(
        "some",
        &config(port, some).replace("\"exit 1\"", "\"exit 0\""),
    );
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("ran api/one.test.ts"), "{out}");
    assert!(!out.contains("ran api/two.test.ts"), "{out}");
    assert!(out.contains("done: api: 1 file not run here"), "{out}");
    let listening = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = listening.local_addr().unwrap().port();
    std::fs::write(
        dir.join("fairlead.toml"),
        config(up, some).replace("\"exit 1\"", "\"exit 0\""),
    )
    .unwrap();
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("ran api/two.test.ts"),
        "a met need runs them: {out}"
    );
    assert!(!out.contains("not run here"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unmet_need_with_no_proved_in_keeps_the_gate_open_and_says_what_to_do() {
    let port = closed_port();
    let unproved =
        "tests = [\"api/**\"]\nchecks = [\"db-lint\"]\nhint = \"Start Postgres on 5432\"";
    let dir = repo("unproved", &config(port, unproved));
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("and the need names no `proved_in`, so the gate can't pass. Meet it (Start Postgres on 5432) or set its `proved_in`"),
        "{out}"
    );
    let why = "`api` wasn't run here because need \"postgres\" isn't met and names no `proved_in`; meet it (`fairlead doctor` checks it) or set its `proved_in` to where it's proved, then run `fairlead done` again";
    assert!(out.contains(&format!("done: not passed: {why}")), "{out}");
    let (code, out) = done(&dir, &["--check"]);
    assert_eq!(code, 1);
    assert_eq!(out.trim(), format!("done: not passed for this tree: {why}"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_ci_settles_an_unmet_need_is_owed_to_ci_and_the_gate_passes_without_proved_in() {
    let port = closed_port();
    let unproved =
        "tests = [\"api/**\"]\nchecks = [\"db-lint\"]\nhint = \"Start Postgres on 5432\"\n\n[ci]\nsettle = true";
    let dir = repo("collected", &config(port, unproved));
    let (code, out) = done(&dir, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("api: 2 files not run here: need \"postgres\" not met"),
        "{out}"
    );
    assert!(
        out.contains("owed to CI, where the merge check collects it. To run them here: Start Postgres on 5432"),
        "{out}"
    );
    assert!(out.contains("(postgres: owed to CI)"), "{out}");
    assert!(
        out.contains("owed to CI before merge: api 2 test files"),
        "{out}"
    );
    let (code, receipt) = run(&dir, env!("CARGO_BIN_EXE_fairlead"), &["receipt"]);
    assert_eq!(code, 0, "{receipt}");
    assert!(
        receipt.contains("api: 2 test files to CI before merge (2 need postgres)"),
        "{receipt}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
