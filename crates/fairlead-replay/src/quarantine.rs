//! Tests declared flaky in named CI jobs. Every failure is still planned and
//! judged; an entry that applies turns its hits, misses and unconfirmed
//! failures into `Quarantined`, keeping what each would have been. An entry
//! applies only until its date and only while the dataset bears it out: the
//! test failed in enough distinct pull requests, and never in another job.

use fairlead_core::config::Quarantine;
use regex::Regex;
use serde::Serialize;

use crate::run::{Failure, Outcome, Target};

/// Distinct pull requests an entry's failures must span before it applies.
pub const MIN_PULLS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Applied, and absorbed at least one failure.
    Active,
    /// Its `until` is before the end of the window, so it doesn't apply.
    Expired,
    /// Too few pull requests, or the test also failed in another job.
    Unverified,
    /// The test didn't fail in the window, or only in ways it doesn't absorb.
    Stale,
}

impl Status {
    /// The name the JSON report uses.
    pub fn name(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Expired => "expired",
            Status::Unverified => "unverified",
            Status::Stale => "stale",
        }
    }
}

/// What one entry did in a replay.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub path: String,
    pub job: String,
    pub reason: String,
    pub until: String,
    pub status: Status,
    /// Distinct pull requests among the test's failures in the window.
    pub pulls: usize,
    /// Jobs the test failed in that the entry doesn't name.
    pub other_jobs: Vec<String>,
    /// Jobs the absorbed failures ran in, which shows how far the regex reaches.
    pub jobs: Vec<String>,
    pub absorbed: usize,
    pub would_hit: usize,
    pub would_miss: usize,
    pub would_unconfirmed: usize,
}

/// Applies each entry to `failures`, judged against the window ending
/// `until`, and reports what each did.
pub fn apply(entries: &[(Quarantine, Regex)], failures: &mut [Failure], until: &str) -> Vec<Entry> {
    entries
        .iter()
        .map(|(q, job)| apply_one(q, job, failures, until))
        .collect()
}

fn apply_one(q: &Quarantine, job: &Regex, failures: &mut [Failure], until: &str) -> Entry {
    let of_test = |f: &Failure| matches!(&f.target, Target::Test(p) if *p == q.path);
    let mut pulls: Vec<u64> = failures
        .iter()
        .filter(|f| of_test(f))
        .filter_map(|f| f.pr)
        .collect();
    pulls.sort_unstable();
    pulls.dedup();
    let mut other_jobs: Vec<String> = failures
        .iter()
        .filter(|f| of_test(f) && !job.is_match(&f.job))
        .map(|f| f.job.clone())
        .collect();
    other_jobs.sort();
    other_jobs.dedup();
    let status = if q.until.as_str() < until {
        Status::Expired
    } else if !failures.iter().any(of_test) {
        Status::Stale
    } else if pulls.len() < MIN_PULLS || !other_jobs.is_empty() {
        Status::Unverified
    } else {
        Status::Active
    };
    let mut entry = Entry {
        path: q.path.clone(),
        job: q.job.clone(),
        reason: q.reason.clone(),
        until: q.until.clone(),
        status,
        pulls: pulls.len(),
        other_jobs,
        jobs: Vec::new(),
        absorbed: 0,
        would_hit: 0,
        would_miss: 0,
        would_unconfirmed: 0,
    };
    if status != Status::Active {
        return entry;
    }
    for f in failures
        .iter_mut()
        .filter(|f| of_test(f) && job.is_match(&f.job))
    {
        match f.outcome {
            Outcome::Hit => entry.would_hit += 1,
            Outcome::Miss => entry.would_miss += 1,
            Outcome::Unconfirmed => entry.would_unconfirmed += 1,
            _ => continue,
        }
        entry.absorbed += 1;
        entry.jobs.push(f.job.clone());
        f.judged = Some(f.outcome);
        f.outcome = Outcome::Quarantined;
    }
    entry.jobs.sort();
    entry.jobs.dedup();
    if entry.absorbed == 0 {
        entry.status = Status::Stale;
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(pr: u64, job: &str, outcome: Outcome) -> Failure {
        Failure {
            run_id: pr,
            attempt: 1,
            event: "pull_request".into(),
            pr: Some(pr),
            head_sha: String::new(),
            job: job.into(),
            target: Target::Test("t.test.ts".into()),
            outcome,
            hit_by: None,
            judged: None,
            detail: String::new(),
            changed: Vec::new(),
        }
    }

    fn entry(job: &str) -> (Quarantine, Regex) {
        let q = Quarantine {
            path: "t.test.ts".into(),
            job: job.into(),
            reason: "flaky".into(),
            until: "2099-01-01".into(),
        };
        (q, Regex::new(job).unwrap())
    }

    #[test]
    fn absorbs_unconfirmed_and_names_the_jobs_it_reached() {
        let mut fs = vec![
            failure(1, "unit linux", Outcome::Unconfirmed),
            failure(2, "unit windows", Outcome::Miss),
            failure(3, "unit windows", Outcome::Flaky),
        ];
        let e = &apply(&[entry("unit")], &mut fs, "2026-09-01")[0];
        assert_eq!(e.status, Status::Active);
        assert_eq!((e.absorbed, e.would_unconfirmed, e.would_miss), (2, 1, 1));
        assert_eq!(e.jobs, ["unit linux", "unit windows"]);
        assert_eq!(fs[0].judged, Some(Outcome::Unconfirmed));
        assert_eq!(
            fs[2].outcome,
            Outcome::Flaky,
            "a flaky failure is left as it is"
        );
    }

    #[test]
    fn stale_when_the_test_never_failed_or_only_flakily() {
        let mut none: Vec<Failure> = Vec::new();
        let e = &apply(&[entry("^win$")], &mut none, "2026-09-01")[0];
        assert_eq!(e.status, Status::Stale);
        let mut flaky: Vec<Failure> = (1..=3)
            .map(|pr| failure(pr, "win", Outcome::Flaky))
            .collect();
        let e = &apply(&[entry("^win$")], &mut flaky, "2026-09-01")[0];
        assert_eq!((e.status, e.pulls, e.absorbed), (Status::Stale, 3, 0));
    }
}
