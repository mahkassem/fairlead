//! `fairlead insights --score` over a planted event log: each line of the
//! text, the JSON, an empty log, a log whose runs predate `reach`, and the
//! flags it can't be combined with.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

use serde_json::{json, Value};

fn fairlead(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.output().unwrap()
}

fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-score-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".git/fairlead")).unwrap();
    let out = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    dir
}

fn done(at: &str, decision: &str, ms: f64, extra: Value) -> String {
    let mut e = json!({"at": at, "stage": "done", "decision": decision, "rules": [], "added": 0, "ms": ms, "fairlead": "t"});
    for (k, v) in extra.as_object().unwrap() {
        e[k] = v.clone();
    }
    e.to_string() + "\n"
}

fn reach(all: bool, reached: usize, suite: usize, ran: usize) -> Value {
    json!({"all": all, "reached": reached, "suite": suite, "ran": ran})
}

const CONFIG: &str = r#"
[[needs]]
id = "smtp"
tcp = ["127.0.0.1:1025"]
tests = ["test/mail/**"]
proved_in = "CI (services job)"

[[needs]]
id = "docker"
command = ["docker", "info"]
tests = ["test/containers/**"]

[[quarantine]]
path = "test/clock.test.ts"
signature = "timezone"
reason = "the clock differs here"
proved_in = "CI on Linux"
until = "2027-01-01"
"#;

/// Five runs in the window and one long before it.
fn plant(dir: &Path) {
    let now = fairlead_guard::events::timestamp(SystemTime::now());
    let log = done(
        "2020-01-01T00:00:00.000Z",
        "fail",
        900_000.0,
        json!({"reach": reach(true, 200, 200, 200)}),
    ) + &done(
        &now,
        "pass",
        20_000.0,
        json!({"reach": reach(true, 200, 200, 200)}),
    ) + &done(
        &now,
        "pass",
        41_000.0,
        json!({
            "reach": reach(false, 40, 200, 10),
            "budget_seconds": 60,
            "debts": [
                {"step": "unit", "files": 24, "why": "budget", "due": "merge"},
                {"step": "e2e", "files": 6, "why": "blanket", "due": "nightly"},
            ],
            "unmet": [
                {"need": "smtp", "step": "unit", "files": 3, "proved": true},
                {"need": "docker", "step": "unit", "files": 7, "proved": false},
            ],
        }),
    ) + &done(
        &now,
        "pass",
        30_000.0,
        json!({
            "reach": reach(false, 20, 200, 15),
            "budget_seconds": 60,
            "debts": [{"step": "unit", "files": 5, "why": "untimed", "due": "merge"}],
            "steps": [
                {"id": "unit", "passed": true, "seconds": 20.0, "files": 15},
                {"id": "clock", "passed": false, "quarantined": true, "seconds": 1.0},
            ],
        }),
    ) + &done(
        &now,
        "fail",
        118_000.0,
        json!({"reach": reach(false, 36, 200, 36)}),
    ) + &done(&now, "deferred", 8_500.0, json!({}));
    std::fs::write(dir.join(".git/fairlead/events.jsonl"), log).unwrap();
    let report = json!({"recall": 1.0, "judged": 36, "misses": [], "recurring_groups": []});
    std::fs::write(dir.join("replay.json"), report.to_string()).unwrap();
}

#[test]
fn score_prints_one_line_per_topic_with_its_numbers_first() {
    let dir = repo("text", CONFIG);
    plant(&dir);
    let out = fairlead(&dir, &["insights", "--score", "--replay", "replay.json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("score  since ")
            && lines[0].ends_with(", 5 gate runs on this machine"),
        "the 2020 run is outside the window: {text}"
    );
    assert_eq!(
        &lines[1..],
        [
            "  plans    1 of 4 selected everything (25%); a median plan reaches 19% of the suite; 1 older run recorded no reach",
            "  gate     median 30 s, p90 118 s; 3 passed, 1 failed, 1 deferred",
            "  budget   2 runs under a budget ran a median 50% of what their plan reached",
            "  debts    35 test files owed in 2 runs: 24 budget, 6 blanket, 5 untimed; due merge 29, nightly 6",
            "  waivers  1 need with proved_in, 1 quarantine entry; 3 files left to proved_in, 1 step not provable here",
            "  misses   0 of 36 failures (recall 100%)",
        ],
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn score_as_json_carries_every_count_under_one_key() {
    let dir = repo("json", CONFIG);
    plant(&dir);
    let out = fairlead(
        &dir,
        &["insights", "--score", "--json", "--replay", "replay.json"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_object().unwrap().keys().collect::<Vec<_>>(), ["score"]);
    let s = &v["score"];
    assert_eq!(
        json!([s["runs"], s["passed"], s["failed"], s["deferred"]]),
        json!([5, 3, 1, 1])
    );
    assert_eq!(
        s["plans"],
        json!({"measured": 4, "everything": 1, "everything_share": 0.25, "reached_share": 0.19, "ran_share": 0.875})
    );
    assert_eq!(
        s["gate"],
        json!({"median_seconds": 30.0, "p90_seconds": 118.0})
    );
    assert_eq!(s["budget"], json!({"runs": 2, "ran_share": 0.5}));
    assert_eq!(
        s["debts"],
        json!({"files": 35, "runs": 2,
            "by_why": {"blanket": 6, "budget": 24, "untimed": 5},
            "by_due": {"merge": 29, "nightly": 6}})
    );
    assert_eq!(
        s["waivers"],
        json!({"needs_proved_in": 1, "quarantine_entries": 1, "files_proved_in": 3, "steps_quarantined": 1})
    );
    assert_eq!(
        s["replay"],
        json!({"failures": 36, "misses": 0, "recall": 1.0})
    );
    let old = fairlead(
        &dir,
        &["insights", "--score", "--json", "--since", "2019-01-01"],
    );
    let v: Value = serde_json::from_slice(&old.stdout).unwrap();
    assert_eq!(v["score"]["since"], "2019-01-01");
    assert_eq!(v["score"]["runs"], 6, "--since widens the window");
    assert!(v["score"].get("replay").is_none(), "no report was given");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_log_says_no_run_was_recorded_and_nothing_else() {
    let dir = repo("empty", "");
    let out = fairlead(&dir, &["insights", "--score", "--since", "2026-10-11"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "score  since 2026-10-11, no gate runs recorded on this machine: `fairlead done` records each one\n"
    );
    let out = fairlead(&dir, &["insights", "--score", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["score"]["runs"], 0);
    assert_eq!(v["score"]["plans"]["everything_share"], Value::Null);
    assert_eq!(v["score"]["gate"]["median_seconds"], Value::Null);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runs_that_predate_reach_are_counted_and_the_plans_line_says_none_carries_it() {
    let dir = repo("older", "");
    let now = fairlead_guard::events::timestamp(SystemTime::now());
    let log = done(
        &now,
        "pass",
        2_000.0,
        json!({"steps": [{"id": "unit", "passed": true, "seconds": 1.5}]}),
    ) + &done(&now, "fail", 4_000.0, json!({}))
        + &done(&now, "pass", 3_000.0, json!({}));
    std::fs::write(dir.join(".git/fairlead/events.jsonl"), log).unwrap();
    let out = fairlead(&dir, &["insights", "--score"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        &lines[1..],
        [
            "  plans    no run in this window recorded what its plan reached; `fairlead done` records it since 0.19.0",
            "  gate     median 3.0 s, p90 4.0 s; 2 passed, 1 failed",
        ],
        "{text}"
    );
    let out = fairlead(&dir, &["insights", "--score", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["score"]["plans"]["measured"], 0);
    assert_eq!(v["score"]["plans"]["reached_share"], Value::Null);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn score_refuses_speed_and_suggest_and_a_report_it_cannot_read() {
    let dir = repo("flags", "");
    for other in ["--speed", "--suggest"] {
        let out = fairlead(&dir, &["insights", "--score", other]);
        assert_eq!(out.status.code(), Some(2), "{other}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("cannot be used with"),
            "{other}"
        );
    }
    let missing = fairlead(&dir, &["insights", "--score", "--replay", "nope.json"]);
    assert_eq!(missing.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}
