//! Token counts from a Claude Code transcript through the Stop hook and from
//! a headless run's JSON: the totals, the fallback when a transcript has no
//! `cost-state`, and that nothing but numbers reaches the event log.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

const CANARY: &str = "CANARY-7f3a-never-logged";

fn fairlead(dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-tokens-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    dir
}

fn lines(values: &[Value]) -> String {
    values.iter().map(|v| v.to_string() + "\n").collect()
}

fn assistant(id: &str, input: u64, output: u64) -> Value {
    json!({"type": "assistant", "isSidechain": false, "message": {"id": id, "model": "m", "content": [{"type": "text", "text": CANARY}],
        "usage": {"input_tokens": input, "output_tokens": output, "cache_read_input_tokens": 10, "cache_creation_input_tokens": 1}}})
}

fn cost_state(start: u64, input: u64, output: u64, cost: f64) -> Value {
    json!({"type": "cost-state", "startTime": start, "totalCostUSD": cost,
        "modelUsage": {"m": {"inputTokens": input, "outputTokens": output, "cacheReadInputTokens": 100, "cacheCreationInputTokens": 20, "costUSD": cost}}})
}

fn stop(dir: &Path, transcript: &Path) -> Output {
    let call = json!({"cwd": dir, "session_id": "s1", "transcript_path": transcript, "stop_hook_active": false});
    fairlead(dir, &["guard", "stop"], &call.to_string())
}

fn report(dir: &Path) -> Vec<Value> {
    let out = fairlead(dir, &["tokens", "report", "--json"], "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice::<Vec<Value>>(&out.stdout).unwrap()
}

#[test]
fn the_stop_hook_records_cost_state_totals_and_nothing_but_numbers() {
    let dir = repo("cost");
    let transcript = dir.join("session.jsonl");
    let user = json!({"type": "user", "message": {"content": CANARY}});
    std::fs::write(
        &transcript,
        lines(&[
            user,
            assistant("a", 5, 1),
            cost_state(1, 100, 10, 1.0),
            assistant("a", 5, 1),
            cost_state(1, 300, 40, 2.5),
            cost_state(2, 50, 5, 0.5),
        ]),
    )
    .unwrap();
    assert!(stop(&dir, &transcript).status.success());
    stop(&dir, &transcript);
    let found = report(&dir);
    assert_eq!(
        found.len(),
        1,
        "a later event for the session replaces the earlier: {found:?}"
    );
    let s = &found[0];
    assert_eq!(
        (
            s["source"].as_str(),
            s["input"].as_u64(),
            s["output"].as_u64()
        ),
        (Some("transcript"), Some(350), Some(45))
    );
    assert_eq!(
        (s["cache_read"].as_u64(), s["cache_write"].as_u64()),
        (Some(200), Some(40))
    );
    assert_eq!(s["cost_usd"], 3.0, "each process's last cost-state, summed");
    assert!(s.get("estimate").is_none());
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    assert!(
        !log.contains(CANARY),
        "transcript text never reaches the log"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_cost_state_usage_is_counted_once_per_message_with_subagents_and_marked_an_estimate() {
    let dir = repo("usage");
    let transcript = dir.join("s1.jsonl");
    std::fs::write(
        &transcript,
        lines(&[
            assistant("a", 5, 1),
            assistant("a", 5, 1),
            assistant("b", 7, 2),
        ]),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("s1/subagents")).unwrap();
    std::fs::write(
        dir.join("s1/subagents/agent-1.jsonl"),
        lines(&[assistant("a", 3, 3)]),
    )
    .unwrap();
    stop(&dir, &transcript);
    let s = &report(&dir)[0];
    assert_eq!(
        (s["input"].as_u64(), s["output"].as_u64()),
        (Some(15), Some(6)),
        "{s}"
    );
    assert_eq!(s["estimate"], true);
    assert!(s.get("cost_usd").is_none(), "no price table, so no cost");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_headless_run_and_the_transcript_it_wrote_agree() {
    let dir = repo("pair");
    let transcript = dir.join("session.jsonl");
    std::fs::write(
        &transcript,
        lines(&[assistant("a", 5, 1), cost_state(1, 300, 40, 2.5)]),
    )
    .unwrap();
    stop(&dir, &transcript);
    let headless = json!({"type": "result", "subtype": "success", "session_id": "s1", "total_cost_usd": 2.5, "result": CANARY,
        "usage": {"input_tokens": 1, "output_tokens": 1},
        "modelUsage": {"m": {"inputTokens": 300, "outputTokens": 40, "cacheReadInputTokens": 100, "cacheCreationInputTokens": 20}}});
    std::fs::write(dir.join("run.json"), headless.to_string()).unwrap();
    let out = fairlead(&dir, &["tokens", "import", "run.json"], "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let found = report(&dir);
    assert_eq!(found.len(), 2);
    let pick = |source: &str| {
        found
            .iter()
            .find(|s| s["source"] == source)
            .unwrap()
            .clone()
    };
    let (a, b) = (pick("transcript"), pick("headless"));
    for key in ["input", "output", "cache_read", "cache_write", "cost_usd"] {
        assert_eq!(a[key], b[key], "{key}");
    }
    assert!(
        !std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl"))
            .unwrap()
            .contains(CANARY)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn events_off_records_no_tokens() {
    let dir = repo("off");
    std::fs::write(dir.join("fairlead.toml"), "[guard]\nevents = \"off\"\n").unwrap();
    let transcript = dir.join("session.jsonl");
    std::fs::write(&transcript, lines(&[cost_state(1, 300, 40, 2.5)])).unwrap();
    stop(&dir, &transcript);
    assert!(report(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
