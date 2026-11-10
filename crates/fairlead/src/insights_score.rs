//! `insights --score`: whether the done gate and its test plans are getting
//! better, from what `fairlead done` records in the event log: how each run
//! ended and how long it took, how much of the suite its plan reached and
//! ran, what a budget left owed to CI, and what a need or a quarantine
//! entry waived. One line per topic, numbers first; nothing is written.

use std::collections::BTreeMap;

use fairlead_core::config::Config;
use serde::Serialize;
use serde_json::Value;

/// The reasons a debt is recorded with, in the order the line names them.
const WHYS: [&str; 3] = ["budget", "blanket", "untimed"];
/// When a debt falls due, soonest first.
const DUES: [&str; 2] = ["merge", "nightly"];

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Score {
    pub since: String,
    /// `done` runs in the window, and how they ended.
    pub runs: usize,
    pub passed: usize,
    pub failed: usize,
    pub deferred: usize,
    pub plans: Plans,
    pub gate: Gate,
    pub budget: Budget,
    pub debts: Debts,
    pub waivers: Waivers,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay: Option<Replay>,
}

/// How wide the plans were. Shares are fractions of 1.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Plans {
    /// Runs that recorded what their plan reached.
    pub measured: usize,
    /// Of those, the plans that selected every test.
    pub everything: usize,
    pub everything_share: Option<f64>,
    /// The median share of the suite a plan reached.
    pub reached_share: Option<f64>,
    /// The median share of what a plan reached that ran here.
    pub ran_share: Option<f64>,
}

/// How long a run of the gate took.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Gate {
    pub median_seconds: Option<f64>,
    pub p90_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Budget {
    /// Runs under a budget.
    pub runs: usize,
    /// The median share of what those runs' plans reached that ran here.
    pub ran_share: Option<f64>,
}

/// Test files reached and left to CI.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Debts {
    pub files: usize,
    /// Runs that left any.
    pub runs: usize,
    pub by_why: BTreeMap<String, usize>,
    pub by_due: BTreeMap<String, usize>,
}

/// What passes without running here: the first two from the config as it
/// is now, the last two from the runs in the window.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Waivers {
    pub needs_proved_in: usize,
    pub quarantine_entries: usize,
    /// Test files an unmet need left to where it says they're proved.
    pub files_proved_in: usize,
    /// Steps that failed as a quarantine entry expects.
    pub steps_quarantined: usize,
}

/// A replay report's failures and the ones its plans missed.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Replay {
    pub failures: u64,
    pub misses: usize,
    pub recall: Option<f64>,
}

/// The middle value, or the mean of the two middle ones of an even count.
fn median(values: &[f64]) -> Option<f64> {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    match v.len() {
        0 => None,
        n if n % 2 == 1 => Some(v[mid]),
        _ => Some((v[mid - 1] + v[mid]) / 2.0),
    }
}

/// The smallest value with at least `p` percent of the values at or under
/// it: the nearest rank, so it is always a value that was measured.
fn percentile(values: &[f64], p: f64) -> Option<f64> {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = (p * v.len() as f64 / 100.0).ceil() as usize;
    v.get(rank.clamp(1, v.len().max(1)) - 1).copied()
}

fn rounded(value: Option<f64>, places: i32) -> Option<f64> {
    let scale = 10f64.powi(places);
    value.map(|v| (v * scale).round() / scale)
}

fn count(v: &Value) -> usize {
    v.as_u64().unwrap_or(0) as usize
}

/// The window's runs, counted, with the samples the medians are taken over.
#[derive(Default)]
struct Tally {
    score: Score,
    seconds: Vec<f64>,
    reached: Vec<f64>,
    ran: Vec<f64>,
    budget_ran: Vec<f64>,
}

impl Tally {
    fn add(&mut self, v: &Value) {
        let s = &mut self.score;
        s.runs += 1;
        match v["decision"].as_str() {
            Some("pass") => s.passed += 1,
            Some("deferred") => s.deferred += 1,
            _ => s.failed += 1,
        }
        if let Some(ms) = v["ms"].as_f64() {
            self.seconds.push(ms / 1000.0);
        }
        let budgeted = v["budget_seconds"].as_u64().is_some_and(|b| b > 0);
        s.budget.runs += usize::from(budgeted);
        let reach = &v["reach"];
        if reach.is_object() {
            s.plans.measured += 1;
            s.plans.everything += usize::from(reach["all"] == true);
            let (reached, suite) = (count(&reach["reached"]), count(&reach["suite"]));
            if suite > 0 {
                self.reached.push(reached as f64 / suite as f64);
            }
            if reached > 0 {
                let share = count(&reach["ran"]) as f64 / reached as f64;
                self.ran.push(share);
                if budgeted {
                    self.budget_ran.push(share);
                }
            }
        }
        let debts = v["debts"].as_array().map_or(&[][..], Vec::as_slice);
        s.debts.runs += usize::from(!debts.is_empty());
        for d in debts {
            let files = count(&d["files"]);
            s.debts.files += files;
            *s.debts.by_why.entry(name(&d["why"])).or_default() += files;
            *s.debts.by_due.entry(name(&d["due"])).or_default() += files;
        }
        s.waivers.files_proved_in += v["unmet"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|u| u["proved"] == true)
            .map(|u| count(&u["files"]))
            .sum::<usize>();
        s.waivers.steps_quarantined += v["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|step| step["quarantined"] == true)
            .count();
    }
}

fn name(v: &Value) -> String {
    v.as_str().unwrap_or("?").to_string()
}

/// The scorecard: the `done` events in the log since `since`, the waivers
/// the config holds, and a replay report's misses when one is given.
pub fn score(log: &str, since: &str, config: &Config, replay: Option<&Value>) -> Score {
    let mut tally = Tally::default();
    for v in log
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
    {
        if v["stage"] == "done" && v["at"].as_str().is_some_and(|a| a >= since) {
            tally.add(&v);
        }
    }
    let mut s = tally.score;
    s.since = since.to_string();
    let share = |n: usize, of: usize| (of > 0).then(|| n as f64 / of as f64);
    s.plans.everything_share = rounded(share(s.plans.everything, s.plans.measured), 4);
    s.plans.reached_share = rounded(median(&tally.reached), 4);
    s.plans.ran_share = rounded(median(&tally.ran), 4);
    s.gate = Gate {
        median_seconds: rounded(median(&tally.seconds), 3),
        p90_seconds: rounded(percentile(&tally.seconds, 90.0), 3),
    };
    s.budget.ran_share = rounded(median(&tally.budget_ran), 4);
    s.waivers.needs_proved_in = config
        .needs
        .items()
        .iter()
        .filter(|n| n.proved_in.is_some())
        .count();
    s.waivers.quarantine_entries = config.quarantine.items().len();
    s.replay = replay.map(|r| Replay {
        failures: r["judged"].as_u64().unwrap_or(0),
        misses: r["misses"].as_array().map_or(0, Vec::len),
        recall: r["recall"].as_f64(),
    });
    s
}

fn pct(share: f64) -> String {
    format!("{:.0}%", share * 100.0)
}

/// To a tenth of a percent unless it is whole, so one miss in many still shows.
fn recall(share: f64) -> String {
    let p = share * 100.0;
    if (p - p.round()).abs() < 0.05 {
        format!("{p:.0}%")
    } else {
        format!("{p:.1}%")
    }
}

/// Whole seconds, and tenths under ten of them.
fn secs(s: f64) -> String {
    if s < 10.0 {
        format!("{s:.1} s")
    } else {
        format!("{s:.0} s")
    }
}

/// `1 run`, `2 runs`: the count with its noun in the right number.
fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn runs(n: usize) -> String {
    counted(n, "run", "runs")
}

/// `800 budget, 380 blanket`: the known names in their order, then any other.
fn parts(counts: &BTreeMap<String, usize>, order: &[&str], number_first: bool) -> String {
    let known = order.iter().filter(|k| counts.contains_key(**k)).copied();
    let other = counts
        .keys()
        .map(String::as_str)
        .filter(|k| !order.contains(k));
    known
        .chain(other)
        .map(|k| {
            if number_first {
                format!("{} {k}", counts[k])
            } else {
                format!("{k} {}", counts[k])
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn plans_line(s: &Score) -> String {
    let p = &s.plans;
    let Some(share) = p.everything_share else {
        return "no run in this window recorded what its plan reached; `fairlead done` records it since 0.19.0".into();
    };
    let mut line = format!(
        "{} of {} selected everything ({})",
        p.everything,
        p.measured,
        pct(share)
    );
    if let Some(reached) = p.reached_share {
        line.push_str(&format!(
            "; a median plan reaches {} of the suite",
            pct(reached)
        ));
    }
    if p.measured < s.runs {
        let older = counted(s.runs - p.measured, "older run", "older runs");
        line.push_str(&format!("; {older} recorded no reach"));
    }
    line
}

fn gate_line(s: &Score) -> String {
    let mut line = String::new();
    if let (Some(median), Some(p90)) = (s.gate.median_seconds, s.gate.p90_seconds) {
        line.push_str(&format!("median {}, p90 {}; ", secs(median), secs(p90)));
    }
    line.push_str(&format!("{} passed, {} failed", s.passed, s.failed));
    if s.deferred > 0 {
        line.push_str(&format!(", {} deferred", s.deferred));
    }
    line
}

/// The score as text: a line per topic that has something to say.
pub fn render(s: &Score) -> String {
    let mut lines: Vec<(&str, String)> = Vec::new();
    if s.runs > 0 {
        lines.push(("plans", plans_line(s)));
        lines.push(("gate", gate_line(s)));
    }
    if s.budget.runs > 0 {
        let mut line = format!("{} under a budget", runs(s.budget.runs));
        if let Some(ran) = s.budget.ran_share {
            line.push_str(&format!(
                " ran a median {} of what their plan reached",
                pct(ran)
            ));
        }
        lines.push(("budget", line));
    }
    if s.debts.runs > 0 {
        lines.push((
            "debts",
            format!(
                "{} owed in {}: {}; due {}",
                counted(s.debts.files, "test file", "test files"),
                runs(s.debts.runs),
                parts(&s.debts.by_why, &WHYS, true),
                parts(&s.debts.by_due, &DUES, false)
            ),
        ));
    }
    let w = &s.waivers;
    if w != &Waivers::default() {
        lines.push((
            "waivers",
            format!(
                "{} with proved_in, {}; {} left to proved_in, {} not provable here",
                counted(w.needs_proved_in, "need", "needs"),
                counted(
                    w.quarantine_entries,
                    "quarantine entry",
                    "quarantine entries"
                ),
                counted(w.files_proved_in, "file", "files"),
                counted(w.steps_quarantined, "step", "steps")
            ),
        ));
    }
    if let Some(r) = &s.replay {
        let recall = r.recall.map_or("-".to_string(), recall);
        lines.push((
            "misses",
            format!("{} of {} failures (recall {recall})", r.misses, r.failures),
        ));
    }
    let mut out = match s.runs {
        0 => format!(
            "score  since {}, no gate runs recorded on this machine: `fairlead done` records each one\n",
            s.since
        ),
        n => format!(
            "score  since {}, {} on this machine\n",
            s.since,
            counted(n, "gate run", "gate runs")
        ),
    };
    for (topic, line) in lines {
        out.push_str(&format!("  {topic:<9}{line}\n"));
    }
    out
}

#[cfg(test)]
#[path = "insights_score_tests.rs"]
mod tests;
