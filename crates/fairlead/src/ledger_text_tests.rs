use super::*;
use fairlead_replay::ledger::{At, Proposal, RunRef};
use fairlead_replay::streaks::Drift;

fn failure(test: &str, verdict: Verdict, why: &str) -> Failure {
    Failure {
        workflow: "ci".into(),
        run_id: 9,
        attempt: 1,
        job: "unit".into(),
        test: Some(test.into()),
        file: Some(test.into()),
        project: None,
        message: "Error: boom".into(),
        fingerprint: "0".into(),
        verdict,
        why: why.into(),
        since: None,
        rerun: None,
    }
}

fn ledger(failures: Vec<Failure>) -> Ledger {
    Ledger {
        version: 1,
        repo: "o/r".into(),
        made_at: "2026-11-08T10:00:00Z".into(),
        head_sha: "abcdef0123".into(),
        base_branch: "main".into(),
        runs: vec![RunRef {
            run_id: 9,
            attempt: 1,
            workflow: "ci".into(),
            event: "pull_request".into(),
            conclusion: "failure".into(),
        }],
        failures,
        proposals: Vec::new(),
        default_branch: Some("main".into()),
        streaks: Vec::new(),
    }
}

const RED: &str = "red on main since abc1234 (2026-11-07)";

#[test]
fn the_summary_leads_with_the_inherited_share_and_its_evidence() {
    let l = ledger(vec![
        failure("a", Verdict::Inherited, RED),
        failure("b", Verdict::Own, "x"),
        failure("c", Verdict::Inherited, RED),
    ]);
    assert_eq!(
        summary(&l),
        "2 of 3 failures inherited: red on main since abc1234 (2026-11-07); 1 own"
    );
    let own = ledger(vec![failure("a", Verdict::Own, "x")]);
    assert_eq!(summary(&own), "all 1 failure own");
    let flaky = ledger(vec![
        failure("a", Verdict::Flaky, "it timed out with no assertion failing"),
        failure("b", Verdict::Infra, "out of memory (exit 137)"),
    ]);
    assert_eq!(
        summary(&flaky),
        "1 of 2 failures flaky: it timed out with no assertion failing; 1 infra"
    );
}

#[test]
fn with_no_failures_the_summary_says_what_ran() {
    let mut l = ledger(Vec::new());
    assert_eq!(summary(&l), "no failures in 1 run");
    l.runs[0].conclusion = "in_progress".into();
    assert_eq!(summary(&l), "no failures so far; 1 run still running");
    l.runs.clear();
    assert_eq!(summary(&l), "no CI runs found for this commit");
}

#[test]
fn a_streak_says_how_long_whether_ever_green_and_any_drift() {
    let s = Streak {
        workflow: "nightly".into(),
        event: "schedule".into(),
        red: 12,
        since: Some(At {
            sha: "abc1234ff".into(),
            date: "2026-10-27".into(),
        }),
        hours: Some(288),
        last_green: None,
        never_green: true,
        whole_history: true,
        runs: 12,
        drift: vec![Drift {
            job: "e2e".into(),
            was: "a: x".into(),
            now: "b: y".into(),
        }],
    };
    assert_eq!(
        streak_line("main", &s).unwrap(),
        "nightly (schedule) red on main for 12 runs since abc1234 (2026-10-27), 288 h; never green since it was added; the cause drifted in e2e: was \"a: x\", now \"b: y\""
    );
    let green = Streak { red: 0, ..s };
    assert_eq!(streak_line("main", &green), None);
}

#[test]
fn the_markdown_says_the_verdicts_once_with_proposals_to_confirm() {
    let mut l = ledger(vec![failure("a|b", Verdict::Flaky, "another attempt on the same commit passed the job")]);
    l.proposals.push(Proposal {
        path: "src/a.test.ts".into(),
        job: "^unit$".into(),
        reason: "flaky".into(),
        until: "2026-11-22".into(),
    });
    let md = markdown(&l);
    assert_eq!(md.matches("#### Is this red mine?").count(), 1);
    assert!(md.contains("| flaky | `unit` | `a\\|b` |"), "{md}");
    assert!(md.contains("[[replay.quarantine]]\npath = \"src/a.test.ts\"\njob = \"^unit$\""), "{md}");
    assert!(text(&l).contains("propose [[replay.quarantine]] path = \"src/a.test.ts\", job = \"^unit$\", reason = \"flaky\", until = \"2026-11-22\""));
}
