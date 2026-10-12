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
