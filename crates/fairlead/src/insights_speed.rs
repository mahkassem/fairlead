//! `insights --speed`: where the time goes, from what Fairlead already
//! records. `done`'s per-step seconds and plan time in the event log, the
//! `ci run --results` files given, and a replay dataset's job times,
//! reruns and cancelled runs. Each line carries its numbers and one change
//! to try; nothing is written.

use std::collections::BTreeMap;
use std::path::PathBuf;

use fairlead_core::config::Config;
use fairlead_replay::dataset::Row;
use serde::Serialize;
use serde_json::Value;

/// Runs a step must pass in a row before a later stage is suggested for it.
pub const NEVER_FAILED: usize = 20;
/// Runs a step must fail in a row before it's called noise.
pub const ALWAYS_FAILS: usize = 5;
/// The slowest steps listed.
const SLOWEST: usize = 3;
/// The pull requests listed for reruns and cancellations.
const PULLS: usize = 3;

/// Where a step's times came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// A `fairlead done` step on this machine.
    Done,
    /// A `ci run` invocation, from a results file.
    Ci,
    /// A CI job, from a replay dataset.
    Job,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Done => "done",
            Source::Ci => "ci run",
            Source::Job => "CI job",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Stat {
    pub id: String,
    pub source: Source,
    pub runs: usize,
    pub failed: usize,
    pub median: f64,
    pub p90: f64,
    pub max: f64,
    pub total: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Line {
    /// `slowest`, `never-failed`, `always-fails`, `run-whole`, `reruns` or `cancelled`.
    pub kind: &'static str,
    pub subject: String,
    pub numbers: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Speed {
    pub steps: Vec<Stat>,
    /// The median time `done` took to plan, or the replay report's.
    pub plan_seconds: Option<f64>,
    pub lines: Vec<Line>,
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

fn stat(id: &str, source: Source, samples: &[(bool, f64)]) -> Stat {
    let mut secs: Vec<f64> = samples.iter().map(|(_, s)| *s).collect();
    secs.sort_by(f64::total_cmp);
    Stat {
        id: id.to_string(),
        source,
        runs: samples.len(),
        failed: samples.iter().filter(|(passed, _)| !passed).count(),
        median: quantile(&secs, 0.5),
        p90: quantile(&secs, 0.9),
        max: *secs.last().expect("a step with samples"),
        total: secs.iter().sum(),
    }
}

/// What the inputs hold, gathered: step samples by source and id, plan
/// times, and the dataset's rows.
#[derive(Default)]
pub struct Inputs {
    pub samples: BTreeMap<(Source, String), Vec<(bool, f64)>>,
    pub plan_seconds: Vec<f64>,
    pub rows: Vec<Row>,
}

impl Inputs {
    /// The `done` events in the log since `since`.
    pub fn add_events(&mut self, log: &str, since: &str) {
        for v in log.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            if v["stage"] != "done" || v["at"].as_str().is_none_or(|a| a < since) {
                continue;
            }
            if let Some(p) = v["plan_seconds"].as_f64() {
                self.plan_seconds.push(p);
            }
            for s in v["steps"].as_array().into_iter().flatten() {
                self.add(Source::Done, s);
            }
        }
    }

    /// The invocations of a `ci run --results` file.
    pub fn add_results(&mut self, results: &Value) {
        for i in results["invocations"].as_array().into_iter().flatten() {
            self.add(Source::Ci, i);
        }
    }

    fn add(&mut self, source: Source, step: &Value) {
        let (Some(id), Some(seconds)) = (step["id"].as_str(), step["seconds"].as_f64()) else {
            return;
        };
        let passed = step["passed"] == true || step["quarantined"] == true;
        self.samples
            .entry((source, id.to_string()))
            .or_default()
            .push((passed, seconds));
    }

    /// A replay dataset's rows: each job's time and outcome.
    pub fn add_rows(&mut self, rows: Vec<Row>) {
        for r in &rows {
            for j in &r.jobs {
                let (Some(seconds), true) = (j.seconds, matches!(j.conclusion.as_str(), "success" | "failure")) else {
                    continue;
                };
                self.samples
                    .entry((Source::Job, j.name.clone()))
                    .or_default()
                    .push((j.conclusion == "success", seconds as f64));
            }
        }
        self.rows.extend(rows);
    }
}

/// The config table a step id belongs to, for a suggestion that names it.
fn table(config: &Config, id: &str) -> Option<&'static str> {
    if config.tests.runners.items().iter().any(|r| r.id == id) {
        Some("tests.runners")
    } else if config.checks.items().iter().any(|c| c.id == id) {
        Some("checks")
    } else {
        None
    }
}

fn secs(s: f64) -> String {
    if s >= 90.0 {
        format!("{:.1} min", s / 60.0)
    } else {
        format!("{s:.1} s")
    }
}

fn subject(s: &Stat) -> String {
    format!("{} ({})", s.id, s.source.name())
}

/// Every line the inputs back.
pub fn coach(inputs: &Inputs, config: &Config, replay_plan: Option<f64>) -> Speed {
    let steps: Vec<Stat> = inputs
        .samples
        .iter()
        .map(|((source, id), samples)| stat(id, *source, samples))
        .collect();
    let plan_seconds = if inputs.plan_seconds.is_empty() {
        replay_plan
    } else {
        let mut p = inputs.plan_seconds.clone();
        p.sort_by(f64::total_cmp);
        Some(quantile(&p, 0.5))
    };
    let mut lines = Vec::new();
    let mut slowest: Vec<&Stat> = steps.iter().collect();
    slowest.sort_by(|a, b| b.total.total_cmp(&a.total));
    for s in slowest.iter().take(SLOWEST).filter(|s| s.total > 0.0) {
        let suggestion = match s.source {
            Source::Done => "it runs on every `fairlead done` that reaches it: a narrower `match`, or tests that import less, keep it out of changes that don't need it".to_string(),
            _ => "split it across jobs with `fairlead ci plan --shards 2`, balanced by `--durations`".to_string(),
        };
        lines.push(Line {
            kind: "slowest",
            subject: subject(s),
            numbers: format!(
                "median {}, p90 {} over {} runs, {} in all",
                secs(s.median),
                secs(s.p90),
                s.runs,
                secs(s.total)
            ),
            suggestion,
        });
    }
    for s in &steps {
        lines.extend(step_lines(s, config, plan_seconds));
    }
    lines.extend(dataset_lines(&inputs.rows));
    Speed {
        steps,
        plan_seconds,
        lines,
    }
}

fn step_lines(s: &Stat, config: &Config, plan: Option<f64>) -> Vec<Line> {
    let mut out = Vec::new();
    let known = table(config, &s.id);
    if s.runs >= NEVER_FAILED && s.failed == 0 && s.source != Source::Job {
        if let Some(table) = known {
            out.push(Line {
                kind: "never-failed",
                subject: subject(s),
                numbers: format!("0 failures in {} runs, {} in all", s.runs, secs(s.total)),
                suggestion: format!(
                    "a candidate for a later stage: `[[{table}]] id = \"{}\", from = \"merge\"` runs it once a change lands instead of on every push",
                    s.id
                ),
            });
        }
    }
    if s.runs >= ALWAYS_FAILS && s.failed == s.runs {
        out.push(Line {
            kind: "always-fails",
            subject: subject(s),
            numbers: format!("failed all {} runs", s.runs),
            suggestion: "its red says nothing any more: fix it, or record what fails today (`[[replay.quarantine]]` for a flaky test in CI, `[[quarantine]]` for one platform, `guard.baseline` for guard findings)".into(),
        });
    }
    let whole_ok = config
        .tests
        .runners
        .items()
        .iter()
        .find(|r| r.id == s.id)
        .is_some_and(|r| r.all_from != Some(fairlead_core::config::CiStage::Draft));
    if let (Some(plan), true, true) = (plan, whole_ok, s.source != Source::Job) {
        if s.runs >= 3 && s.max <= plan {
            out.push(Line {
                kind: "run-whole",
                subject: subject(s),
                numbers: format!(
                    "its longest run took {} against {} to plan",
                    secs(s.max),
                    secs(plan)
                ),
                suggestion: format!(
                    "choosing its tests saves less than planning costs: `[[tests.runners]] id = \"{}\", all_from = \"draft\"` runs it whole at every stage",
                    s.id
                ),
            });
        }
    }
    out
}

/// Reruns and cancellations from a replay dataset, overall and by pull request.
fn dataset_lines(rows: &[Row]) -> Vec<Line> {
    if rows.is_empty() {
        return Vec::new();
    }
    let runs: BTreeMap<u64, Vec<&Row>> = rows.iter().fold(BTreeMap::new(), |mut m, r| {
        m.entry(r.run_id).or_default().push(r);
        m
    });
    let mut by_pull: BTreeMap<u64, (usize, usize, usize)> = BTreeMap::new();
    let (mut reruns, mut rescued, mut cancelled) = (0, 0, 0);
    for attempts in runs.values() {
        let rerun = attempts.iter().filter(|r| r.attempt > 1).count();
        let saved = attempts.iter().any(|r| r.conclusion == "failure")
            && attempts
                .iter()
                .max_by_key(|r| r.attempt)
                .is_some_and(|r| r.conclusion == "success" && r.attempt > 1);
        let stopped = attempts.iter().filter(|r| r.cancelled()).count();
        reruns += rerun;
        rescued += usize::from(saved);
        cancelled += stopped;
        if let Some(pr) = attempts[0].pr {
            let e = by_pull.entry(pr).or_default();
            e.0 += 1;
            e.1 += rerun;
            e.2 += stopped;
        }
    }
    let mut worst: Vec<(&u64, &(usize, usize, usize))> =
        by_pull.iter().filter(|(_, c)| c.1 + c.2 > 0).collect();
    worst.sort_by(|a, b| (b.1 .1 + b.1 .2).cmp(&(a.1 .1 + a.1 .2)).then(a.0.cmp(b.0)));
    let pulls: Vec<String> = worst
        .iter()
        .take(PULLS)
        .map(|(pr, c)| format!("#{pr}: {} runs, {} reruns, {} cancelled", c.0, c.1, c.2))
        .collect();
    let mut out = Vec::new();
    let share = |n: usize| (n * 100) as f64 / runs.len() as f64;
    if reruns > 0 {
        out.push(Line {
            kind: "reruns",
            subject: "reruns".into(),
            numbers: format!(
                "{reruns} reruns over {} runs; {rescued} passed only on a rerun of the same commit ({:.0}%)",
                runs.len(),
                share(rescued)
            ),
            suggestion: "a run that passes on a rerun is a flaky test or the platform: `fairlead ci ledger` names which, and proposes a `[[replay.quarantine]]` entry for a flaky test".into(),
        });
    }
    if cancelled > 0 {
        out.push(Line {
            kind: "cancelled",
            subject: "cancelled runs".into(),
            numbers: format!(
                "{cancelled} of {} runs cancelled ({:.0}%)",
                runs.len(),
                share(cancelled)
            ),
            suggestion: "pushes in quick succession cancel each other's runs: keep the pull request a draft while pushing often, so only the steps `from = \"draft\"` run".into(),
        });
    }
    if !pulls.is_empty() {
        out.push(Line {
            kind: "pulls",
            subject: "most rerun or cancelled".into(),
            numbers: pulls.join("; "),
            suggestion: "look at these first: a pull request that keeps rerunning has a flaky test or a red base in its way".into(),
        });
    }
    out
}

/// The speed section as text.
pub fn render(s: &Speed) -> String {
    let mut out = String::from("speed\n");
    match s.plan_seconds {
        Some(p) => out.push_str(&format!("  plan     median {} to plan a change\n", secs(p))),
        None => out.push_str("  plan     no plan time recorded yet (`fairlead done` records it)\n"),
    }
    if s.lines.is_empty() {
        out.push_str("  nothing recorded yet: `fairlead done`, `ci run --results` or a replay dataset (`--dataset`) gives it numbers\n");
    }
    for l in &s.lines {
        out.push_str(&format!(
            "  {:<13}{}: {}\n    → {}\n",
            l.kind, l.subject, l.numbers, l.suggestion
        ));
    }
    out
}

/// Reads the replay datasets named.
pub fn rows(paths: &[PathBuf]) -> Result<Vec<Row>, String> {
    let mut all = Vec::new();
    for p in paths {
        if !p.is_file() {
            return Err(format!("could not read {}", p.display()));
        }
        all.extend(fairlead_replay::dataset::read(p)?);
    }
    Ok(all)
}

#[cfg(test)]
#[path = "insights_speed_tests.rs"]
mod tests;
