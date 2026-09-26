//! The replay report: recall over attributed failures, what else each
//! failure turned out to be, how much each plan selected, and each miss
//! with the rule that would have caught it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::run::{Failure, HitBy, Outcome, Replayed, Target};
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

/// Recall for the runs of one event, such as `merge_group`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EventRecall {
    pub hits: usize,
    pub misses: usize,
    pub unconfirmed: usize,
    pub recall: Option<f64>,
    pub strict_recall: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub repo: String,
    pub from: String,
    pub until: String,
    pub runs: usize,
    pub hits: usize,
    /// Hits by how the target got into the plan.
    pub hits_selected: usize,
    pub hits_run_all: usize,
    pub hits_check: usize,
    pub misses: Vec<Miss>,
    pub flaky: usize,
    pub unconfirmed: usize,
    pub unattributed: usize,
    pub unavailable: usize,
    pub errors: usize,
    /// Failed jobs no rule watches, by job name.
    pub unwatched: BTreeMap<String, usize>,
    pub ignored: usize,
    /// Hits over hits and misses; `None` with nothing attributed.
    pub recall: Option<f64>,
    /// Hits over hits, misses and unconfirmed failures.
    pub strict_recall: Option<f64>,
    pub by_event: BTreeMap<String, EventRecall>,
    /// The attributed-failure gate from `replay.min_failures`.
    pub min_failures: u32,
    pub median_selected: Option<f64>,
    pub p90_selected: Option<f64>,
    pub run_all_share: Option<f64>,
    pub median_plan_seconds: Option<f64>,
    pub p90_plan_seconds: Option<f64>,
    /// The first plan's time, with the parse cache cold.
    pub first_plan_seconds: Option<f64>,
}

fn ratio(hits: usize, of: usize) -> Option<f64> {
    (of > 0).then(|| hits as f64 / of as f64)
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
    let hit_by = |by: HitBy| {
        replayed
            .failures
            .iter()
            .filter(|f| f.hit_by == Some(by))
            .count()
    };
    let mut unwatched = BTreeMap::new();
    for f in replayed
        .failures
        .iter()
        .filter(|f| f.outcome == Outcome::Unwatched)
    {
        *unwatched.entry(f.job.clone()).or_insert(0) += 1;
    }
    let mut by_event: BTreeMap<String, EventRecall> = BTreeMap::new();
    for f in &replayed.failures {
        let e = by_event.entry(f.event.clone()).or_default();
        match f.outcome {
            Outcome::Hit => e.hits += 1,
            Outcome::Miss => e.misses += 1,
            Outcome::Unconfirmed => e.unconfirmed += 1,
            _ => {}
        }
    }
    by_event.retain(|_, e| e.hits + e.misses + e.unconfirmed > 0);
    for e in by_event.values_mut() {
        e.recall = ratio(e.hits, e.hits + e.misses);
        e.strict_recall = ratio(e.hits, e.hits + e.misses + e.unconfirmed);
    }
    let unconfirmed = count(Outcome::Unconfirmed);
    let seconds: Vec<f64> = replayed.plans.iter().map(|p| p.seconds).collect();
    Report {
        repo: repo.to_string(),
        from: window.from.clone(),
        until: window.until.clone(),
        runs: replayed.runs,
        hits,
        hits_selected: hit_by(HitBy::Selected),
        hits_run_all: hit_by(HitBy::RunAll),
        hits_check: hit_by(HitBy::Check),
        flaky: count(Outcome::Flaky),
        unconfirmed,
        unattributed: count(Outcome::Unattributed),
        unavailable: count(Outcome::Unavailable),
        errors: count(Outcome::Error),
        unwatched,
        ignored: count(Outcome::Ignored),
        recall: ratio(hits, judged),
        strict_recall: ratio(hits, judged + unconfirmed),
        by_event,
        min_failures,
        median_selected: quantile(ratios.clone(), 0.5),
        p90_selected: quantile(ratios, 0.9),
        run_all_share: (plans > 0)
            .then(|| replayed.plans.iter().filter(|p| p.all).count() as f64 / plans as f64),
        median_plan_seconds: quantile(seconds.clone(), 0.5),
        p90_plan_seconds: quantile(seconds.clone(), 0.9),
        first_plan_seconds: seconds.first().copied(),
        misses,
    }
}

fn pct(v: Option<f64>) -> String {
    v.map_or("-".into(), |v| format!("{:.1}%", v * 100.0))
}

fn secs(v: Option<f64>) -> String {
    v.map_or("-".into(), |s| format!("{s:.2} s"))
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
        "  runs {}  attributed failures {judged} (gate {}: {gate})  recall {}  strict recall {}",
        r.runs,
        r.min_failures,
        pct(r.recall),
        pct(r.strict_recall)
    );
    let _ = writeln!(
        out,
        "  hits {} (selected {}, run_all {}, checks {})  misses {}",
        r.hits,
        r.hits_selected,
        r.hits_run_all,
        r.hits_check,
        r.misses.len()
    );
    let _ = writeln!(
        out,
        "  flaky {}  unconfirmed {}  unattributed {}  unavailable {}  errors {}  ignored {}",
        r.flaky, r.unconfirmed, r.unattributed, r.unavailable, r.errors, r.ignored
    );
    for (event, e) in &r.by_event {
        let _ = writeln!(
            out,
            "  {event}: hits {}  misses {}  unconfirmed {}  recall {}  strict {}",
            e.hits,
            e.misses,
            e.unconfirmed,
            pct(e.recall),
            pct(e.strict_recall)
        );
    }
    let _ = writeln!(
        out,
        "  selected: median {}  p90 {}  run_all {}",
        pct(r.median_selected),
        pct(r.p90_selected),
        pct(r.run_all_share)
    );
    let _ = writeln!(
        out,
        "  plan time: first {}  median {}  p90 {}",
        secs(r.first_plan_seconds),
        secs(r.median_plan_seconds),
        secs(r.p90_plan_seconds)
    );
    if !r.unwatched.is_empty() {
        let _ = writeln!(out, "  unwatched failed jobs (no rule names them):");
        for (job, n) in &r.unwatched {
            let _ = writeln!(out, "    {n:>4}  {job}");
        }
    }
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
