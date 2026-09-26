//! What the event log says about the hooks, for `fairlead doctor`: how the
//! write hook decided, which rules denied, how long it took, and the commit
//! stage's runs.

use std::collections::BTreeMap;

use serde_json::Value;

/// Write-hook calls read, newest first.
pub const WINDOW: usize = 500;

#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub writes: usize,
    pub decisions: BTreeMap<String, usize>,
    pub denies: BTreeMap<String, usize>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub commits: usize,
    pub commit_decisions: BTreeMap<String, usize>,
}

/// Nearest rank, so the value is one that was measured.
fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// The newest `WINDOW` write-hook calls, and every commit-stage run, from
/// the log's lines. A line that isn't an event is skipped.
pub fn summarize(log: &str) -> Summary {
    let events: Vec<Value> = log
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let stage = |e: &Value, s: &str| e.get("stage").and_then(Value::as_str) == Some(s);
    let decision = |e: &Value| {
        e.get("decision")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    };
    let mut summary = Summary::default();
    let writes: Vec<&Value> = events
        .iter()
        .rev()
        .filter(|e| stage(e, "write"))
        .take(WINDOW)
        .collect();
    let mut ms: Vec<f64> = Vec::new();
    for e in &writes {
        let d = decision(e);
        if d == "deny" {
            for rule in e
                .get("rules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                *summary.denies.entry(rule.to_string()).or_default() += 1;
            }
        }
        *summary.decisions.entry(d).or_default() += 1;
        if let Some(t) = e.get("ms").and_then(Value::as_f64) {
            ms.push(t);
        }
    }
    ms.sort_by(f64::total_cmp);
    summary.writes = writes.len();
    summary.p50 = percentile(&ms, 50.0);
    summary.p95 = percentile(&ms, 95.0);
    for e in events.iter().filter(|e| stage(e, "commit")) {
        summary.commits += 1;
        *summary.commit_decisions.entry(decision(e)).or_default() += 1;
    }
    summary
}

fn counts(map: &BTreeMap<String, usize>) -> String {
    let mut pairs: Vec<(&String, &usize)> = map.iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    pairs
        .iter()
        .map(|(k, n)| format!("{n} {k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn render(s: &Summary) -> String {
    let mut out = String::new();
    if s.writes == 0 {
        out.push_str("write hook: no calls recorded\n");
    } else {
        let window = if s.writes == WINDOW {
            format!(" (the last {WINDOW})")
        } else {
            String::new()
        };
        out.push_str(&format!(
            "write hook: {} call(s){window}: {}\n",
            s.writes,
            counts(&s.decisions)
        ));
        if let (Some(p50), Some(p95)) = (s.p50, s.p95) {
            out.push_str(&format!("  latency: p50 {p50:.1} ms, p95 {p95:.1} ms\n"));
        }
        if !s.denies.is_empty() {
            out.push_str(&format!("  denied by: {}\n", counts(&s.denies)));
        }
        for (key, what) in [("timed_out", "ran out of time"), ("error", "hit an error")] {
            if let Some(n) = s.decisions.get(key) {
                out.push_str(&format!("  {n} call(s) {what} and let the edit through\n"));
            }
        }
    }
    if s.commits == 0 {
        out.push_str("commit stage: no runs recorded\n");
    } else {
        out.push_str(&format!(
            "commit stage: {} run(s): {}\n",
            s.commits,
            counts(&s.commit_decisions)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(decision: &str, ms: f64, rules: &[&str]) -> String {
        serde_json::json!({ "stage": "write", "decision": decision, "ms": ms, "rules": rules })
            .to_string()
    }

    #[test]
    fn a_synthetic_log_renders_decisions_latency_denies_and_failures() {
        let mut lines: Vec<String> = (1..=20).map(|i| write("allow", i as f64, &[])).collect();
        lines.push(write("deny", 30.0, &["file-length", "item-code"]));
        lines.push(write("deny", 31.0, &["item-code"]));
        lines.push(write("timed_out", 40.0, &[]));
        lines.push(write("error", 1.0, &[]));
        lines.push(r#"{"stage":"commit","decision":"allow","ms":7.0,"rules":[]}"#.into());
        lines.push(r#"{"stage":"commit","decision":"deny","ms":9.0,"rules":["history"]}"#.into());
        lines.push("not an event".into());
        let out = render(&summarize(&lines.join("\n")));
        assert_eq!(
            out,
            "write hook: 24 call(s): 20 allow, 2 deny, 1 error, 1 timed_out\n\
             \x20 latency: p50 11.0 ms, p95 31.0 ms\n\
             \x20 denied by: 2 item-code, 1 file-length\n\
             \x20 1 call(s) ran out of time and let the edit through\n\
             \x20 1 call(s) hit an error and let the edit through\n\
             commit stage: 2 run(s): 1 allow, 1 deny\n"
        );
    }

    #[test]
    fn only_the_newest_window_of_writes_counts() {
        let mut lines: Vec<String> = (0..WINDOW).map(|_| write("allow", 1.0, &[])).collect();
        lines.insert(0, write("deny", 99.0, &["old"]));
        let s = summarize(&lines.join("\n"));
        assert_eq!((s.writes, s.denies.get("old")), (WINDOW, None));
        assert!(render(&s).contains("(the last 500)"));
    }

    #[test]
    fn an_empty_log_says_nothing_was_recorded() {
        assert_eq!(
            render(&summarize("")),
            "write hook: no calls recorded\ncommit stage: no runs recorded\n"
        );
    }
}
