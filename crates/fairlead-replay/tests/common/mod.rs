//! Helpers the replay tests share: a git history to replay, and the rows
//! and jobs a dataset records.
#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

use fairlead_replay::dataset::{Job, Row};

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

pub fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

pub fn job(name: &str, conclusion: &str, log: &[&str]) -> Job {
    Job {
        name: name.into(),
        conclusion: conclusion.into(),
        failed_steps: Vec::new(),
        annotations: Vec::new(),
        annotations_capped: false,
        log: log.iter().map(|s| s.to_string()).collect(),
        image: None,
        seconds: None,
    }
}

pub fn row(
    run_id: u64,
    attempt: u32,
    pr: u64,
    head: &str,
    base: &str,
    day: u32,
    jobs: Vec<Job>,
) -> Row {
    let conclusion = if jobs.iter().any(Job::failed) {
        "failure"
    } else {
        "success"
    };
    Row {
        repo: "example/repo".into(),
        run_id,
        attempt,
        event: "pull_request".into(),
        workflow: "ci".into(),
        pr: Some(pr),
        head_sha: head.into(),
        base_sha: Some(base.into()),
        created_at: format!("2026-09-{day:02}T10:00:00Z"),
        conclusion: conclusion.into(),
        seconds: None,
        jobs,
    }
}

pub const CONFIG: &str = r#"
[[tests.runners]]
id = "vitest"
match = ["**"]
command = ["vitest", "run", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["playwright", "test", "{files}"]

[[replay.failures]]
runner = "vitest"
extractor = "vitest"
job = "^test$"

[replay]
ignore = ["^all-green$"]
ignore_steps = ["^Install$"]
"#;
