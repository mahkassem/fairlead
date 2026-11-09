//! The red ledger: each failing test or check of a CI run with its
//! fingerprint and a verdict on whether the change caused it. `infra` when
//! the job failed before any test ran or the log carries an infrastructure
//! mark; `flaky` when another attempt on the same tree passed the job
//! (replay's `Flaky`) or a test timed out with no assertion failing;
//! `inherited` when the base branch's own runs of the workflow failed alike
//! (replay's evidence from the base); otherwise `own`.

use std::collections::BTreeSet;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::dataset::{Job, Row};
use crate::extract::{extract, Extractor, Printed};
use crate::fingerprint::{id, message_for, test_id};
use crate::signals;

/// The ledger file's version.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Nothing outside the change explains it.
    Own,
    /// The base branch fails it alike.
    Inherited,
    /// It passed on the same tree, or timed out with no assertion failing.
    Flaky,
    /// It failed before any test ran, or the platform failed under it.
    Infra,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Own => "own",
            Verdict::Inherited => "inherited",
            Verdict::Flaky => "flaky",
            Verdict::Infra => "infra",
        }
    }
}

/// A commit on the base branch and when its run started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct At {
    pub sha: String,
    /// `YYYY-MM-DD`.
    pub date: String,
}

impl At {
    /// Where and when a run ran.
    pub fn of_row(row: &Row) -> At {
        At {
            sha: row.head_sha.clone(),
            date: row.created_at.get(..10).unwrap_or("").to_string(),
        }
    }

    /// `abc1234 (2026-11-07)`.
    pub fn short(&self) -> String {
        format!(
            "{} ({})",
            self.sha.chars().take(7).collect::<String>(),
            self.date
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub workflow: String,
    pub run_id: u64,
    pub attempt: u32,
    pub job: String,
    /// The test as the log named it, `path > title`; none for a job or
    /// check that failed with no test named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
    /// The test file as the log printed it, for a rerun to find.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The failure's message with run-to-run noise taken out.
    pub message: String,
    pub fingerprint: String,
    pub verdict: Verdict,
    /// The evidence, in a phrase: "red on main since abc1234 (2026-11-07)".
    pub why: String,
    /// For an inherited failure, the oldest base run of the red streak that has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<At>,
    /// What `ci ledger --rerun` found, when it ran this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rerun: Option<String>,
}

/// One failure as a run's job shows it, before a verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub job: String,
    pub printed: Option<Printed>,
    pub test: Option<String>,
    pub message: String,
    pub fingerprint: String,
}

/// The extractors for a job: those whose job pattern matches its name.
pub type Extractors = [(Regex, Extractor)];

/// The failures a failed job names: each test its annotations or log name,
/// or the job itself when none is named.
pub fn found_in(job: &Job, extractors: &Extractors) -> Vec<Found> {
    let mut printed: Vec<Printed> = job
        .annotations
        .iter()
        .filter(|a| !a.path.is_empty())
        .map(|a| Printed {
            path: a.path.clone(),
            project: None,
            title: Some(a.title.clone()).filter(|t| !t.is_empty()),
        })
        .collect();
    let log = job.log.join("\n");
    for (_, extractor) in extractors.iter().filter(|(re, _)| re.is_match(&job.name)) {
        for p in extract(extractor, &log) {
            if !printed
                .iter()
                .any(|q| q.path == p.path && q.title == p.title)
            {
                printed.push(p);
            }
        }
    }
    if printed.is_empty() {
        let message = message_for(&job.log, None);
        let message = if message.is_empty() {
            job.failed_steps.join(", ")
        } else {
            message
        };
        return vec![Found {
            fingerprint: id(&job.name, "", &message),
            job: job.name.clone(),
            printed: None,
            test: None,
            message,
        }];
    }
    printed
        .into_iter()
        .map(|p| {
            let test = test_id(&p);
            let message = message_for(&job.log, Some(&p));
            Found {
                fingerprint: id(&job.name, &test, &message),
                job: job.name.clone(),
                test: Some(test),
                message,
                printed: Some(p),
            }
        })
        .collect()
}

/// Every failure of a run's failed jobs.
pub fn found_in_row(row: &Row, extractors: &Extractors) -> Vec<Found> {
    row.jobs
        .iter()
        .filter(|j| j.failed())
        .flat_map(|j| found_in(j, extractors))
        .collect()
}

/// What the ledger reads to judge one run.
pub struct Evidence<'a> {
    /// Other attempts of the run, and other runs of the workflow on the same commit.
    pub same_tree: &'a [Row],
    /// The base branch's runs of the workflow, newest first.
    pub base: &'a [Row],
    pub base_branch: &'a str,
    pub extractors: &'a Extractors,
}

/// Whether a base run failed alike: the same fingerprint, or for a named
/// test the same test in the same job.
fn same(a: &Found, b: &Found) -> bool {
    a.fingerprint == b.fingerprint || (a.test.is_some() && a.job == b.job && a.test == b.test)
}

/// The base runs' failures, newest first, beside whether each ran the job.
fn base_failures<'a>(ev: &'a Evidence, job: &str) -> Vec<(&'a Row, Option<Vec<Found>>)> {
    ev.base
        .iter()
        .filter_map(|r| {
            let j = r.jobs.iter().find(|j| j.name == job)?;
            Some((r, j.failed().then(|| found_in(j, ev.extractors))))
        })
        .collect()
}

/// The inherited reading of `f`, when the base runs bear it out.
fn inherited(f: &Found, ev: &Evidence) -> Option<(String, At)> {
    let runs = base_failures(ev, &f.job);
    let first = runs.iter().position(|(_, found)| {
        found
            .as_ref()
            .is_some_and(|fs| fs.iter().any(|b| same(f, b)))
    })?;
    let mut oldest = first;
    while oldest + 1 < runs.len()
        && runs[oldest + 1]
            .1
            .as_ref()
            .is_some_and(|fs| fs.iter().any(|b| same(f, b)))
    {
        oldest += 1;
    }
    let since = At::of_row(runs[oldest].0);
    let base = ev.base_branch;
    let why = if first == 0 {
        format!("red on {base} since {}", since.short())
    } else {
        format!(
            "red on {base} from {}, green again at {}: update the branch",
            since.short(),
            At::of_row(runs[0].0).short()
        )
    };
    Some((why, since))
}

/// Whether another attempt or run on the same commit passed the job.
fn passed_on_same_tree(row: &Row, job: &str, ev: &Evidence) -> bool {
    let job = Job {
        name: job.to_string(),
        conclusion: String::new(),
        failed_steps: Vec::new(),
        annotations: Vec::new(),
        annotations_capped: false,
        log: Vec::new(),
        image: None,
        seconds: None,
    };
    let all: Vec<&Row> = ev.same_tree.iter().collect();
    crate::run::passed_on_another_attempt(row, &job, &all)
        || ev.same_tree.iter().any(|o| {
            o.run_id != row.run_id
                && o.head_sha == row.head_sha
                && o.workflow == row.workflow
                && o.jobs
                    .iter()
                    .any(|j| j.name == job.name && j.conclusion == "success")
        })
}

/// Every failure of `row` with its verdict.
pub fn judge(row: &Row, ev: &Evidence) -> Vec<Failure> {
    let mut out = Vec::new();
    for job in row.jobs.iter().filter(|j| j.failed()) {
        let found = found_in(job, ev.extractors);
        let named = found.iter().any(|f| f.test.is_some());
        let infra = signals::infra(job, named);
        let rerun_passed = passed_on_same_tree(row, &job.name, ev);
        let timed_out = signals::timed_out_without_assertion(&job.log);
        for f in found {
            let (verdict, why, since) = if let Some(why) = &infra {
                (Verdict::Infra, why.clone(), None)
            } else if rerun_passed {
                (
                    Verdict::Flaky,
                    "another attempt on the same commit passed the job".to_string(),
                    None,
                )
            } else if let Some((why, since)) = inherited(&f, ev) {
                (Verdict::Inherited, why, Some(since))
            } else if timed_out
                || signals::is_timeout(&f.message) && !signals::is_assertion(&f.message)
            {
                (
                    Verdict::Flaky,
                    "it timed out with no assertion failing".to_string(),
                    None,
                )
            } else {
                (
                    Verdict::Own,
                    format!("not failing on {}, and no rerun passed it", ev.base_branch),
                    None,
                )
            };
            out.push(Failure {
                workflow: row.workflow.clone(),
                run_id: row.run_id,
                attempt: row.attempt,
                job: f.job.clone(),
                file: f.printed.as_ref().map(|p| p.path.clone()),
                project: f.printed.as_ref().and_then(|p| p.project.clone()),
                test: f.test.clone(),
                message: f.message.clone(),
                fingerprint: f.fingerprint.clone(),
                verdict,
                why,
                since,
                rerun: None,
            });
        }
    }
    out
}

/// The distinct fingerprints a run's failed job gave, for drift.
pub fn fingerprints(row: &Row, job: &str, extractors: &Extractors) -> BTreeSet<String> {
    row.jobs
        .iter()
        .filter(|j| j.name == job && j.failed())
        .flat_map(|j| found_in(j, extractors))
        .map(|f| f.fingerprint)
        .collect()
}

/// A run the ledger read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRef {
    pub run_id: u64,
    pub attempt: u32,
    pub workflow: String,
    pub event: String,
    /// `failure`, `success`, `cancelled` or `in_progress`.
    pub conclusion: String,
}

/// A `[[replay.quarantine]]` entry the ledger proposes for a flaky test,
/// for a person to confirm and add; never written by Fairlead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub path: String,
    /// The CI job, as an anchored regex.
    pub job: String,
    pub reason: String,
    pub until: String,
}

impl Proposal {
    /// The entry as config text, as `insights --suggest` prints one.
    pub fn entry(&self) -> String {
        format!(
            "[[replay.quarantine]] path = {:?}, job = {:?}, reason = {:?}, until = \"{}\"",
            self.path, self.job, self.reason, self.until
        )
    }
}

/// What `fairlead ci ledger` writes: the failures of the runs it read,
/// each with its verdict, the quarantine entries the flaky ones back, and
/// the default branch's red streaks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u32,
    pub repo: String,
    /// When it was made, RFC 3339 in UTC.
    pub made_at: String,
    /// The commit whose runs were judged.
    pub head_sha: String,
    /// The branch failures were compared with.
    pub base_branch: String,
    pub runs: Vec<RunRef>,
    pub failures: Vec<Failure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposals: Vec<Proposal>,
    /// The branch the streaks are for, when they were read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub streaks: Vec<crate::streaks::Streak>,
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;
