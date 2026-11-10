use super::*;
use serde_json::json;

const AT: &str = "2026-11-05T10:00:00.000Z";

fn done(decision: &str, ms: f64, extra: Value) -> String {
    let mut e = json!({"at": AT, "stage": "done", "decision": decision, "ms": ms});
    for (k, v) in extra.as_object().unwrap() {
        e[k] = v.clone();
    }
    e.to_string() + "\n"
}

fn reach(all: bool, reached: usize, suite: usize, ran: usize) -> Value {
    json!({"all": all, "reached": reached, "suite": suite, "ran": ran})
}

#[test]
fn the_median_is_the_middle_value_or_the_mean_of_the_two_middle_ones() {
    assert_eq!(median(&[]), None);
    assert_eq!(median(&[7.0]), Some(7.0));
    assert_eq!(median(&[9.0, 1.0, 5.0]), Some(5.0));
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
}

#[test]
fn a_percentile_is_the_nearest_rank_so_always_a_value_that_was_measured() {
    assert_eq!(percentile(&[], 90.0), None);
    assert_eq!(percentile(&[3.0], 90.0), Some(3.0));
    let ten: Vec<f64> = (1..=10).rev().map(f64::from).collect();
    assert_eq!(percentile(&ten, 90.0), Some(9.0));
    assert_eq!(percentile(&ten, 91.0), Some(10.0));
    assert_eq!(percentile(&ten, 50.0), Some(5.0));
    assert_eq!(percentile(&ten, 0.0), Some(1.0));
    assert_eq!(percentile(&ten, 100.0), Some(10.0));
    let twenty: Vec<f64> = (1..=20).map(f64::from).collect();
    assert_eq!(percentile(&twenty, 90.0), Some(18.0));
}

#[test]
fn runs_are_counted_by_how_they_ended_and_only_inside_the_window() {
    let log = done("pass", 1000.0, json!({}))
        + &done("fail", 3000.0, json!({}))
        + &done("deferred", 2000.0, json!({}))
        + &json!({"at": "2026-01-01T00:00:00.000Z", "stage": "done", "decision": "fail", "ms": 9.0})
            .to_string()
        + "\n"
        + &json!({"at": AT, "stage": "write", "decision": "allow", "ms": 9.0}).to_string()
        + "\nnot json\n";
    let s = score(&log, "2026-11-01", &Config::default(), None);
    assert_eq!((s.runs, s.passed, s.failed, s.deferred), (3, 1, 1, 1));
    assert_eq!(
        (s.gate.median_seconds, s.gate.p90_seconds),
        (Some(2.0), Some(3.0))
    );
    assert_eq!(s.plans, Plans::default(), "none of them carries a reach");
}

#[test]
fn plan_shares_are_medians_over_the_runs_that_can_give_one() {
    let log = done("pass", 1.0, json!({"reach": reach(true, 100, 100, 100)}))
        + &done(
            "pass",
            1.0,
            json!({"reach": reach(false, 10, 100, 5), "budget_seconds": 60}),
        )
        + &done(
            "pass",
            1.0,
            json!({"reach": reach(false, 20, 100, 5), "budget_seconds": 60}),
        )
        + &done("pass", 1.0, json!({"reach": reach(false, 0, 0, 0)}))
        + &done("pass", 1.0, json!({"budget_seconds": 60}));
    let s = score(&log, "2026-11-01", &Config::default(), None);
    assert_eq!((s.plans.measured, s.plans.everything), (4, 1));
    assert_eq!(s.plans.everything_share, Some(0.25));
    assert_eq!(
        s.plans.reached_share,
        Some(0.2),
        "of 1.0, 0.1 and 0.2: an empty suite gives no share"
    );
    assert_eq!(s.plans.ran_share, Some(0.5), "of 1.0, 0.5 and 0.25");
    assert_eq!(
        (s.budget.runs, s.budget.ran_share),
        (3, Some(0.375)),
        "the budgeted run with no reach counts as a run and gives no share"
    );
}

#[test]
fn debts_and_waivers_add_up_files_by_reason_and_by_when_they_fall_due() {
    let config: Config = toml::from_str(
        r#"
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
"#,
    )
    .unwrap();
    let log = done(
        "pass",
        1.0,
        json!({
            "debts": [
                {"step": "unit", "files": 30, "why": "budget", "due": "merge"},
                {"step": "e2e", "files": 12, "why": "blanket", "due": "nightly"},
            ],
            "unmet": [
                {"need": "smtp", "step": "unit", "files": 4, "proved": true},
                {"need": "docker", "step": "unit", "files": 9, "proved": false},
            ],
            "steps": [
                {"id": "unit", "passed": true, "seconds": 1.0},
                {"id": "clock", "passed": false, "quarantined": true, "seconds": 1.0},
            ],
        }),
    ) + &done(
        "pass",
        1.0,
        json!({"debts": [{"step": "unit", "files": 8, "why": "untimed", "due": "merge"}]}),
    ) + &done("pass", 1.0, json!({}));
    let s = score(&log, "2026-11-01", &config, None);
    assert_eq!((s.debts.files, s.debts.runs), (50, 2));
    assert_eq!(
        serde_json::to_value(&s.debts.by_why).unwrap(),
        json!({"blanket": 12, "budget": 30, "untimed": 8})
    );
    assert_eq!(
        serde_json::to_value(&s.debts.by_due).unwrap(),
        json!({"merge": 38, "nightly": 12})
    );
    assert_eq!(
        s.waivers,
        Waivers {
            needs_proved_in: 1,
            quarantine_entries: 1,
            files_proved_in: 4,
            steps_quarantined: 1,
        }
    );
    let text = render(&s);
    assert!(
        text.contains("  debts    50 test files owed in 2 runs: 30 budget, 12 blanket, 8 untimed; due merge 38, nightly 12\n"),
        "{text}"
    );
    assert!(
        text.contains("  waivers  1 need with proved_in, 1 quarantine entry; 4 files left to proved_in, 1 step not provable here\n"),
        "{text}"
    );
}

#[test]
fn a_replay_report_gives_failures_misses_and_recall_as_insights_reads_them() {
    let report =
        json!({"recall": 0.972, "judged": 36, "misses": [{"fix": "a"}], "recurring_groups": []});
    let s = score("", "2026-11-01", &Config::default(), Some(&report));
    assert_eq!(
        s.replay,
        Some(Replay {
            failures: 36,
            misses: 1,
            recall: Some(0.972),
        })
    );
    assert!(
        render(&s).contains("  misses   1 of 36 failures (recall 97.2%)\n"),
        "{}",
        render(&s)
    );
    let whole = json!({"recall": 1.0, "judged": 36, "misses": []});
    let s = score("", "2026-11-01", &Config::default(), Some(&whole));
    assert!(
        render(&s).contains("misses   0 of 36 failures (recall 100%)"),
        "{}",
        render(&s)
    );
}

#[test]
fn a_line_with_nothing_to_say_is_left_out() {
    let s = score("", "2026-11-01", &Config::default(), None);
    assert_eq!(
        render(&s),
        "score  since 2026-11-01, no gate runs recorded on this machine: `fairlead done` records each one\n"
    );
    let log = done(
        "pass",
        41_000.0,
        json!({"reach": reach(false, 18, 100, 18)}),
    );
    let s = score(&log, "2026-11-01", &Config::default(), None);
    assert_eq!(
        render(&s),
        "score  since 2026-11-01, 1 gate run on this machine\n  plans    0 of 1 selected everything (0%); a median plan reaches 18% of the suite\n  gate     median 41 s, p90 41 s; 1 passed, 0 failed\n"
    );
}
