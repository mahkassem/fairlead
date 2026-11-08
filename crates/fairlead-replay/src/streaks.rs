//! Red streaks on the default branch, one per workflow and event, so a
//! scheduled suite red for weeks shows as plainly as a red pull request:
//! how many runs in a row failed and since when, whether the workflow was
//! ever green in the runs read, and whether the newest red run fails for
//! another reason than the one before it (the cause drifted).

use serde::{Deserialize, Serialize};

use crate::dataset::Row;
use crate::ledger::{fingerprints, found_in, At, Extractors};
use crate::window::epoch_seconds;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Streak {
    pub workflow: String,
    /// `push`, `schedule` or another event the branch's runs came from.
    pub event: String,
    /// Red runs in a row, newest first; 0 when the newest run passed.
    pub red: usize,
    /// The first run of the red streak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<At>,
    /// Hours from the streak's first run to when the ledger was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<u64>,
    /// The newest run that passed, when one was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_green: Option<At>,
    /// No run read passed.
    pub never_green: bool,
    /// The runs read are every run the workflow has had on the branch, so
    /// `never_green` means never since it was added.
    pub whole_history: bool,
    pub runs: usize,
    /// Jobs red in both of the newest two red runs whose failures differ.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drift: Vec<Drift>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drift {
    pub job: String,
    /// The earlier red run's first failure, `test: message`.
    pub was: String,
    pub now: String,
}

/// Red: any job failed. A cancelled run says nothing either way.
fn red(row: &Row) -> bool {
    row.conclusion == "failure"
}

fn first_failure(row: &Row, job: &str, extractors: &Extractors) -> String {
    row.jobs
        .iter()
        .filter(|j| j.name == job && j.failed())
        .flat_map(|j| found_in(j, extractors))
        .next()
        .map(|f| match f.test {
            Some(t) => format!("{t}: {}", f.message),
            None => f.message,
        })
        .unwrap_or_default()
}

/// The jobs red in both runs whose fingerprints differ.
fn drift(newer: &Row, older: &Row, extractors: &Extractors) -> Vec<Drift> {
    newer
        .jobs
        .iter()
        .filter(|j| j.failed() && !j.log.is_empty())
        .filter(|j| older.jobs.iter().any(|o| o.name == j.name && o.failed() && !o.log.is_empty()))
        .filter(|j| {
            fingerprints(newer, &j.name, extractors) != fingerprints(older, &j.name, extractors)
        })
        .map(|j| Drift {
            job: j.name.clone(),
            was: first_failure(older, &j.name, extractors),
            now: first_failure(newer, &j.name, extractors),
        })
        .collect()
}

/// The streak of one workflow and event from its runs, newest first.
/// `whole_history` says the runs are all it has had; `now` is a timestamp.
pub fn streak(runs: &[Row], whole_history: bool, now: &str, extractors: &Extractors) -> Option<Streak> {
    let runs: Vec<&Row> = runs.iter().filter(|r| !r.cancelled()).collect();
    let newest = runs.first()?;
    let red_runs: Vec<&Row> = runs.iter().copied().take_while(|r| red(r)).collect();
    let since = red_runs.last().map(|r| At::of_row(r));
    let hours = red_runs.last().and_then(|r| {
        let span = epoch_seconds(now)? - epoch_seconds(&r.created_at)?;
        u64::try_from(span / 3_600).ok()
    });
    let last_green = runs.iter().find(|r| !red(r)).map(|r| At::of_row(r));
    let drift = match red_runs.as_slice() {
        [newer, older, ..] => drift(newer, older, extractors),
        _ => Vec::new(),
    };
    Some(Streak {
        workflow: newest.workflow.clone(),
        event: newest.event.clone(),
        red: red_runs.len(),
        since,
        hours,
        never_green: last_green.is_none(),
        last_green,
        whole_history,
        runs: runs.len(),
        drift,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Job;
    use crate::extract::Extractor;
    use regex::Regex;

    fn run(id: u64, day: u32, conclusion: &str, log: &[&str]) -> Row {
        let failed = conclusion == "failure";
        Row {
            repo: "o/r".into(),
            run_id: id,
            attempt: 1,
            event: "schedule".into(),
            workflow: "nightly".into(),
            pr: None,
            head_sha: format!("c{id:06}"),
            base_sha: None,
            created_at: format!("2026-11-{day:02}T19:17:00Z"),
            conclusion: conclusion.into(),
            seconds: None,
            jobs: vec![Job {
                name: "e2e".into(),
                conclusion: if failed { "failure" } else { "success" }.into(),
                failed_steps: Vec::new(),
                annotations: Vec::new(),
                annotations_capped: false,
                log: log.iter().map(|s| s.to_string()).collect(),
                image: None,
                seconds: None,
            }],
        }
    }

    fn x() -> Vec<(Regex, Extractor)> {
        vec![(Regex::new("").unwrap(), Extractor::Vitest)]
    }

    #[test]
    fn a_red_streak_counts_back_to_the_last_green_run() {
        let runs = vec![
            run(5, 7, "failure", &[" FAIL  e2e/a.test.ts > a", "Error: x"]),
            run(4, 6, "cancelled", &[]),
            run(3, 5, "failure", &[" FAIL  e2e/a.test.ts > a", "Error: x"]),
            run(2, 4, "success", &[]),
        ];
        let s = streak(&runs, false, "2026-11-08T19:17:00Z", &x()).unwrap();
        assert_eq!((s.red, s.runs, s.never_green), (2, 3, false));
        assert_eq!(s.since.unwrap().short(), "c000003 (2026-11-05)");
        assert_eq!(s.hours, Some(72));
        assert_eq!(s.last_green.unwrap().date, "2026-11-04");
        assert!(s.drift.is_empty(), "the same failure both times");
    }

    #[test]
    fn a_workflow_never_green_and_a_cause_that_drifted_are_both_said() {
        let runs = vec![
            run(3, 7, "failure", &[" FAIL  e2e/b.test.ts > b", "Error: y"]),
            run(2, 6, "failure", &[" FAIL  e2e/a.test.ts > a", "Error: x"]),
        ];
        let s = streak(&runs, true, "2026-11-08T00:00:00Z", &x()).unwrap();
        assert!(s.never_green && s.whole_history);
        assert_eq!(s.drift.len(), 1);
        assert_eq!(s.drift[0].was, "e2e/a.test.ts > a: Error: x");
        assert_eq!(s.drift[0].now, "e2e/b.test.ts > b: Error: y");
        let green = streak(&[run(4, 8, "success", &[])], false, "2026-11-08T20:00:00Z", &x()).unwrap();
        assert_eq!((green.red, green.since, green.hours), (0, None, None));
        assert!(streak(&[], false, "2026-11-08T00:00:00Z", &x()).is_none());
    }
}
