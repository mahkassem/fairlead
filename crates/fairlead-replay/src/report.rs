//! The replay report: recall over attributed failures, what else each
//! failure turned out to be, how much each plan selected, and each miss
//! with the rule that would have caught it.

use std::fmt::Write as _;

use serde::Serialize;

use crate::run::{Failure, Outcome, Replayed, Target};
use crate::window::Window;

#[derive(Debug, Clone, Serialize)]
pub struct Miss {
    pub run_id: u64,
    pub attempt: u32,
    pub pr: Option<u64>,
    pub head_sha: String,
    pub target: String,
    pub changed: Vec<String>,
    /// An owner rule that would have selected it.
    pub fix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub repo: String,
    pub from: String,
    pub until: String,
    pub runs: usize,
    pub hits: usize,
    pub misses: Vec<Miss>,
    pub flaky: usize,
    pub unconfirmed: usize,
    pub unattributed: usize,
    pub unavailable: usize,
    pub errors: usize,
    /// Hits over hits and misses; `None` with nothing attributed.
    pub recall: Option<f64>,
    /// The attributed-failure gate from `replay.min_failures`.
    pub min_failures: u32,
    pub median_selected: Option<f64>,
    pub p90_selected: Option<f64>,
    pub run_all_share: Option<f64>,
    pub median_plan_seconds: Option<f64>,
}

fn quantile(mut values: Vec<f64>, q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() - 1) as f64 * q).round() as usize;
    Some(values[index])
}

fn dir_glob(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/**"),
        None => "**".into(),
    }
}

fn miss(f: &Failure) -> Miss {
    let target = match &f.target {
        Target::Test(p) => p.clone(),
        Target::Check(id) => format!("check {id}"),
        Target::Job => format!("job {}", f.job),
    };
    let fix = match (&f.target, f.changed.first()) {
        (Target::Test(test), Some(changed)) => format!(
            "[[tests.owners]] match = \"{}\", covers = [\"{}\"]",
            dir_glob(test),
            dir_glob(changed)
        ),
        (Target::Check(id), Some(changed)) => {
            format!("add \"{}\" to the paths of check {id}", dir_glob(changed))
        }
        _ => "no rule suggested".into(),
    };
    Miss {
        run_id: f.run_id,
        attempt: f.attempt,
        pr: f.pr,
        head_sha: f.head_sha.clone(),
        target,
        changed: f.changed.clone(),
        fix,
    }
}

pub fn report(repo: &str, window: &Window, min_failures: u32, replayed: &Replayed) -> Report {
    let count = |o: Outcome| replayed.failures.iter().filter(|f| f.outcome == o).count();
    let hits = count(Outcome::Hit);
    let misses: Vec<Miss> = replayed
        .failures
        .iter()
        .filter(|f| f.outcome == Outcome::Miss)
        .map(miss)
        .collect();
    let judged = hits + misses.len();
    let ratios: Vec<f64> = replayed
        .plans
        .iter()
        .filter(|p| p.total > 0)
        .map(|p| {
            if p.all {
                1.0
            } else {
                p.selected as f64 / p.total as f64
            }
        })
        .collect();
    let plans = replayed.plans.len();
    Report {
        repo: repo.to_string(),
        from: window.from.clone(),
        until: window.until.clone(),
        runs: replayed.runs,
        hits,
        misses,
        flaky: count(Outcome::Flaky),
        unconfirmed: count(Outcome::Unconfirmed),
        unattributed: count(Outcome::Unattributed),
        unavailable: count(Outcome::Unavailable),
        errors: count(Outcome::Error),
        recall: (judged > 0).then(|| hits as f64 / judged as f64),
        min_failures,
        median_selected: quantile(ratios.clone(), 0.5),
        p90_selected: quantile(ratios, 0.9),
        run_all_share: (plans > 0)
            .then(|| replayed.plans.iter().filter(|p| p.all).count() as f64 / plans as f64),
        median_plan_seconds: quantile(replayed.plans.iter().map(|p| p.seconds).collect(), 0.5),
    }
}

fn pct(v: Option<f64>) -> String {
    v.map_or("-".into(), |v| format!("{:.1}%", v * 100.0))
}

pub fn text(r: &Report) -> String {
    let mut out = String::new();
    let judged = r.hits + r.misses.len();
    let gate = if judged as u32 >= r.min_failures {
        "met"
    } else {
        "not met"
    };
    let _ = writeln!(out, "{} ({} to {})", r.repo, r.from, r.until);
    let _ = writeln!(
        out,
        "  runs {}  attributed failures {judged} (gate {}: {gate})  recall {}",
        r.runs,
        r.min_failures,
        pct(r.recall)
    );
    let _ = writeln!(
        out,
        "  flaky {}  unconfirmed {}  unattributed {}  unavailable {}  errors {}",
        r.flaky, r.unconfirmed, r.unattributed, r.unavailable, r.errors
    );
    let _ = writeln!(
        out,
        "  selected: median {}  p90 {}  run_all {}  plan time median {}",
        pct(r.median_selected),
        pct(r.p90_selected),
        pct(r.run_all_share),
        r.median_plan_seconds
            .map_or("-".into(), |s| format!("{s:.2} s"))
    );
    for m in &r.misses {
        let _ = writeln!(
            out,
            "\n  miss  run {} attempt {} (PR {})  {}",
            m.run_id,
            m.attempt,
            m.pr.map_or("-".into(), |p| p.to_string()),
            m.target
        );
        let _ = writeln!(out, "        changed: {}", m.changed.join(", "));
        let _ = writeln!(out, "        fix: {}", m.fix);
    }
    out
}
