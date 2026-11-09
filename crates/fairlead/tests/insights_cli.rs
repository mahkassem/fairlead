//! `fairlead insights` over a planted event log, replay report and CI
//! results: the counts, the window, the suggestions, and `--write` changing
//! one key with the file's comments kept.

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
    let dir = std::env::temp_dir().join(format!("fairlead-insights-{name}-{}", std::process::id()));
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

fn event(at: &str, stage: &str, decision: &str, extra: Value) -> String {
    let mut e = json!({"at": at, "stage": stage, "decision": decision, "rules": [], "added": 0, "ms": 1.0, "fairlead": "t"});
    for (k, v) in extra.as_object().unwrap() {
        e[k] = v.clone();
    }
    e.to_string() + "\n"
}

const FIX: &str = "[[tests.owners]] match = \"test/**\", covers = [\"src/forms/**\"]";

fn plant(dir: &Path) {
    let now = fairlead_guard::events::timestamp(SystemTime::now());
    let mut log = event(
        "2020-01-01T00:00:00.000Z",
        "done",
        "fail",
        json!({"steps": [{"id": "old", "passed": false, "seconds": 1.0}]}),
    );
    for _ in 0..25 {
        log += &event(&now, "write", "warn", json!({}));
    }
    log += &event(&now, "write", "deny", json!({"rules": ["size"]}));
    log += &event(&now, "done", "pass", json!({}));
    log += &event(
        &now,
        "done",
        "fail",
        json!({"steps": [{"id": "unit", "passed": false, "seconds": 2.0}, {"id": "lint", "passed": true, "seconds": 1.0}]}),
    );
    log += &event(
        &now,
        "tokens",
        "transcript",
        json!({"session": "s1", "tokens": {"input": 1, "output": 2, "cache_read": 3, "cache_write": 4, "cost_usd": 1.5}}),
    );
    log += &event(
        &now,
        "tokens",
        "transcript",
        json!({"session": "s1", "tokens": {"input": 9, "output": 9, "cache_read": 9, "cache_write": 9, "cost_usd": 2.0}}),
    );
    std::fs::write(dir.join(".git/fairlead/events.jsonl"), log).unwrap();
    let report = json!({"recall": 0.9, "judged": 20, "misses": [{"fix": FIX}, {"fix": "no rule suggested"}],
        "recurring_groups": [{"path": "test/flaky.test.ts", "job": "unit (node 20)", "pulls": [1, 2, 3]}]});
    std::fs::write(dir.join("replay.json"), report.to_string()).unwrap();
    let results = json!({"judged": [{"runner": "vitest", "test": "test/forms.test.ts", "verdict": "escaped", "fix": FIX}]});
    std::fs::write(dir.join("results.json"), results.to_string()).unwrap();
}

#[test]
fn insights_count_the_window_and_suggest_only_what_the_evidence_backs() {
    let dir = repo(
        "all",
        "# keep this comment\n[guard]\non_finding = \"warn\" # for now\nbudget_ms = 60\n",
    );
    plant(&dir);
    let out = fairlead(
        &dir,
        &[
            "insights",
            "--json",
            "--suggest",
            "--replay",
            "replay.json",
            "--results",
            "results.json",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let i: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(i["guard"]["writes"], 26);
    assert_eq!(i["guard"]["denies"]["size"], 1);
    assert_eq!(
        (i["done"]["passed"].as_u64(), i["done"]["failed"].as_u64()),
        (Some(1), Some(1)),
        "the 2020 run is outside the window"
    );
    assert_eq!(i["done"]["failing_steps"], json!({"unit": 1}));
    assert_eq!(
        (
            i["tokens"]["sessions"].as_u64(),
            i["tokens"]["cost_usd"].as_f64()
        ),
        (Some(1), Some(2.0)),
        "the newest record replaces the earlier"
    );
    assert_eq!(i["escapes"], 1);
    let changes: Vec<&str> = i["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["change"].as_str().unwrap())
        .collect();
    assert!(
        changes.contains(&FIX),
        "a miss and an escape needed the same rule: {changes:?}"
    );
    assert!(
        changes.iter().any(|c| c.starts_with(
            "[[replay.quarantine]] path = \"test/flaky.test.ts\", job = \"^unit \\(node 20\\)$\""
        )),
        "{changes:?}"
    );
    assert!(changes.contains(&"[guard] on_finding = \"deny\""));
    let writes: Vec<bool> = i["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["writes"].as_bool().unwrap())
        .collect();
    assert_eq!(
        writes.iter().filter(|w| **w).count(),
        1,
        "only the guard flip is written"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_flips_one_key_and_keeps_the_file_as_it_was() {
    let config = "# keep this comment\n[guard]\non_finding = \"warn\" # for now\nbudget_ms = 60\n\n[tests]\nunreached = \"all\"\n";
    let dir = repo("write", config);
    plant(&dir);
    let out = fairlead(&dir, &["insights", "--suggest", "--write"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    assert_eq!(text, config.replace("\"warn\"", "\"deny\""));
    let check = fairlead(&dir, &["config", "check"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    let again = fairlead(&dir, &["insights", "--suggest", "--json"]);
    let i: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert!(
        i["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["writes"] == false),
        "nothing left to write"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_guard_with_no_table_gets_one_and_a_dotted_key_is_edited_in_place() {
    let dir = repo("dotted", "guard.on_finding = \"warn\"\n");
    plant(&dir);
    assert!(fairlead(&dir, &["insights", "--suggest", "--write"])
        .status
        .success());
    assert_eq!(
        std::fs::read_to_string(dir.join("fairlead.toml")).unwrap(),
        "guard.on_finding = \"deny\"\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn speed_reads_done_steps_results_and_a_dataset_and_prints_numbers_with_a_suggestion() {
    let dir = repo(
        "speed",
        "[[tests.runners]]\nid = \"unit\"\nmatch = [\"test/**\"]\ncommand = [\"sh\", \"{files}\"]\n",
    );
    let now = fairlead_guard::events::timestamp(SystemTime::now());
    let mut log = String::new();
    for _ in 0..3 {
        log += &event(
            &now,
            "done",
            "pass",
            json!({"steps": [{"id": "unit", "passed": true, "seconds": 0.3}], "plan_seconds": 1.2}),
        );
    }
    std::fs::write(dir.join(".git/fairlead/events.jsonl"), log).unwrap();
    let rows = [
        json!({"repo": "o/r", "run_id": 1, "attempt": 1, "event": "pull_request", "workflow": "ci", "pr": 4,
               "head_sha": "h", "created_at": "2026-11-01T00:00:00Z", "conclusion": "failure",
               "jobs": [{"name": "test", "conclusion": "failure", "seconds": 300}]}),
        json!({"repo": "o/r", "run_id": 1, "attempt": 2, "event": "pull_request", "workflow": "ci", "pr": 4,
               "head_sha": "h", "created_at": "2026-11-01T00:00:00Z", "conclusion": "success",
               "jobs": [{"name": "test", "conclusion": "success", "seconds": 280}]}),
        json!({"repo": "o/r", "run_id": 2, "attempt": 1, "event": "pull_request", "workflow": "ci", "pr": 4,
               "head_sha": "h2", "created_at": "2026-11-01T01:00:00Z", "conclusion": "cancelled", "jobs": []}),
    ];
    let text: String = rows.iter().map(|r| r.to_string() + "\n").collect();
    std::fs::write(dir.join("runs.jsonl"), text).unwrap();
    let out = fairlead(&dir, &["insights", "--speed", "--dataset", "runs.jsonl"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(
        text.contains("plan     median 1.2 s to plan a change"),
        "{text}"
    );
    assert!(
        text.contains("slowest      test (CI job): median 5.0 min"),
        "{text}"
    );
    assert!(
        text.contains("run-whole    unit (done): its longest run took 0.3 s against 1.2 s to plan"),
        "{text}"
    );
    assert!(
        text.contains("1 passed only on a rerun of the same commit (50%)"),
        "{text}"
    );
    assert!(text.contains("1 of 2 runs cancelled (50%)"), "{text}");
    assert!(text.contains("    → "), "{text}");
    let out = fairlead(
        &dir,
        &["insights", "--speed", "--dataset", "runs.jsonl", "--json"],
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["speed"]["plan_seconds"], 1.2);
    assert!(v["speed"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["kind"] == "cancelled"));
    let missing = fairlead(&dir, &["insights", "--speed", "--dataset", "nope.jsonl"]);
    assert_eq!(missing.status.code(), Some(2));
}

#[test]
fn suggest_proposes_replay_quarantine_for_the_flaky_tests_a_ledger_found() {
    let dir = repo("ledger", "");
    let ledger = json!({"version": 1, "repo": "o/r", "made_at": fairlead_guard::events::timestamp(SystemTime::now()),
        "head_sha": "abc", "base_branch": "main", "runs": [], "failures": [],
        "proposals": [{"path": "test/c.test.ts", "job": "^e2e$", "reason": "it timed out with no assertion failing in run 9 on 2026-11-08", "until": "2026-11-22"}]});
    std::fs::create_dir_all(dir.join(".git/fairlead/ledger")).unwrap();
    std::fs::write(
        dir.join(".git/fairlead/ledger/abc.json"),
        ledger.to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("given.json"), ledger.to_string()).unwrap();
    let out = fairlead(&dir, &["insights", "--suggest", "--ledger", "given.json"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert_eq!(
        text.matches("[[replay.quarantine]] path = \"test/c.test.ts\", job = \"^e2e$\"")
            .count(),
        1,
        "{text}"
    );
    assert!(text.contains("found it flaky in 2 ledger(s)"), "{text}");
}
