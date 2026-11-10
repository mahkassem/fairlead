//! The budget in `fairlead done`: the gate runs what fits in
//! `done.budget_seconds`, the tests nearest the change first, and records
//! what else the change reaches as debts for CI. A gate that fits the
//! machine it runs on can always pass honestly, and nothing reached is
//! forgotten: it is owed, by name and count, where a merge can check it.

use std::collections::{BTreeMap, BTreeSet};

use fairlead_core::config::{Config, Phase, Runner};
use fairlead_core::pattern::Pattern;
use fairlead_core::plan::{Invocation, InvocationKind, Plan, Reason, Status, TestSelection};
use fairlead_guard::events::{Debt, Reach};
use fairlead_lang::EdgeKind;
use fairlead_tests::modules::Modules;
use fairlead_tests::{invoke, nightly, testfiles};

use crate::done_history::History;
use crate::done_needs::tests_of;
use crate::plan_cmd::Planned;

/// Test files a runner with no recorded time runs on its first budgeted run.
const TRIAL: usize = 20;

/// How near a test is to the change, nearest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    /// The test file changed, or it runs in every plan.
    Changed,
    /// It imports a changed file.
    Direct,
    /// An owner rule names it for a changed path.
    Owner,
    /// Its runner's smoke set, for a suite reached only through a blanket.
    Smoke,
    /// It reaches the change over this many files.
    Chain(usize),
    /// Only a `run_all` path, an unreached file's widening or a wide
    /// `[[graph.edges]]` rule reaches it.
    Blanket,
}

/// What the budget made of the gate's steps.
#[derive(Debug, Default)]
pub struct Cut {
    pub steps: Vec<Invocation>,
    pub debts: Vec<Debt>,
    /// One line for each runner that owes something.
    pub lines: Vec<String>,
    pub ran: usize,
}

/// The modules and each runner's suite size, which the plan doesn't carry.
pub struct Suites {
    modules: Modules,
    sizes: BTreeMap<String, usize>,
    tests: BTreeSet<String>,
}

impl Suites {
    pub fn read(planned: &Planned) -> Result<Suites, String> {
        let scan = &planned.scan;
        let modules = Modules::discover(&scan.tree, &scan.packages, &planned.config.modules)?;
        let found = testfiles::discover(&scan.tree, &planned.config, &modules)?;
        let runners = planned.config.tests.runners.items();
        let mut sizes = BTreeMap::new();
        for test in &found.tests {
            if let Some(runner) = test.runner.and_then(|i| runners.get(i)) {
                *sizes.entry(runner.id.clone()).or_insert(0) += 1;
            }
        }
        let tests = found.tests.into_iter().map(|t| t.path).collect();
        Ok(Suites {
            modules,
            sizes,
            tests,
        })
    }

    pub fn total(&self) -> usize {
        self.tests.len()
    }
}

/// The plan in numbers, for the record: `ran` test files of those reached.
pub fn reach(planned: &Planned, suites: &Suites, ran: usize) -> Reach {
    Reach {
        all: planned.plan.all,
        reached: planned.plan.tests.len(),
        suite: suites.total(),
        ran,
    }
}

/// Whether the plan widened past what the change reaches by a file, so a
/// plan without the widening says how near each test really is.
pub fn widened(plan: &Plan) -> bool {
    plan.all
        || plan
            .tests
            .iter()
            .any(|t| matches!(t.reason, Reason::RunAll { .. } | Reason::Unreached { .. }))
}

fn smoke(runner: &Runner) -> Result<Vec<Pattern>, String> {
    runner.smoke.iter().map(|g| Pattern::new(g)).collect()
}

/// Each planned test's class. `narrow` is the plan without `run_all` and
/// the unreached widening: a test it leaves out is reached by a blanket.
pub fn classes(
    planned: &Planned,
    narrow: &Plan,
    suites: &Suites,
) -> Result<BTreeMap<String, Class>, String> {
    let graph = &planned.scan.graph;
    let config = &planned.config;
    let near: BTreeMap<&str, &TestSelection> =
        narrow.tests.iter().map(|t| (t.path.as_str(), t)).collect();
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &narrow.tests {
        if let (Some(runner), true) = (t.runner.as_deref(), nightly::through_rule(graph, &t.reason))
        {
            *by_rule.entry(runner).or_insert(0) += 1;
        }
    }
    let wide = |runner: &str| {
        let suite = suites.sizes.get(runner).copied().unwrap_or(0) as f64;
        let n = by_rule.get(runner).copied().unwrap_or(0) as f64;
        n > 0.0 && n >= config.plan.blanket_share * suite
    };
    let mut smokes: BTreeMap<&str, Vec<Pattern>> = BTreeMap::new();
    for runner in config.tests.runners.items() {
        smokes.insert(runner.id.as_str(), smoke(runner)?);
    }
    let mut out = BTreeMap::new();
    for test in &planned.plan.tests {
        let runner = test.runner.as_deref().unwrap_or("");
        let reason = near.get(test.path.as_str()).map(|t| &t.reason);
        let class = match reason {
            Some(Reason::Changed | Reason::Canary) => Class::Changed,
            Some(Reason::Owner { .. }) => Class::Owner,
            Some(r @ Reason::Import { chain }) => {
                if nightly::through_rule(graph, r) && wide(runner) {
                    Class::Blanket
                } else if chain.len() <= 2 {
                    Class::Direct
                } else {
                    Class::Chain(chain.len())
                }
            }
            _ if test.reason == Reason::Canary => Class::Changed,
            _ => Class::Blanket,
        };
        let stands = smokes
            .get(runner)
            .is_some_and(|globs| globs.iter().any(|g| g.is_match(&test.path)));
        let class = match (class, stands) {
            (Class::Blanket, true) => Class::Smoke,
            (class, _) => class,
        };
        out.insert(test.path.clone(), class);
    }
    Ok(out)
}

/// Each runner's seconds for one test file: the median over its recent
/// passing `done` steps that recorded how many files they named.
pub fn seconds_per_file(log: &str) -> BTreeMap<String, f64> {
    let mut seen: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for line in log.lines().filter(|l| l.contains("\"stage\":\"done\"")) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        for step in event["steps"].as_array().into_iter().flatten() {
            let (Some(id), Some(seconds), Some(files)) = (
                step["id"].as_str(),
                step["seconds"].as_f64(),
                step["files"].as_u64(),
            ) else {
                continue;
            };
            if files > 0 && step["passed"] == true {
                seen.entry(id.to_string())
                    .or_default()
                    .push(seconds / files as f64);
            }
        }
    }
    seen.into_iter()
        .map(|(id, mut all)| {
            let recent = all.split_off(all.len().saturating_sub(10));
            (id, median(recent))
        })
        .collect()
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    match values.len() {
        0 => 0.0,
        n => values[n / 2],
    }
}

fn budgeted(step: &Invocation) -> bool {
    step.kind == InvocationKind::Runner && step.quarantined.is_none()
}

/// The plan's runner steps, each naming its test files, so an unmet need
/// and the budget can each leave files out. A held test keeps its own step.
pub fn named(planned: &Planned, suites: &Suites) -> Vec<Invocation> {
    let alone: BTreeSet<String> = planned
        .plan
        .invocations
        .iter()
        .filter(|s| s.kind == InvocationKind::Runner && s.quarantined.is_some())
        .flat_map(|s| tests_of(planned, s))
        .collect();
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for step in &planned.plan.invocations {
        if !budgeted(step) {
            out.push(step.clone());
            continue;
        }
        if !seen.insert(step.id.clone()) {
            continue;
        }
        let tests: Vec<&TestSelection> = planned
            .plan
            .tests
            .iter()
            .filter(|t| t.runner.as_deref() == Some(step.id.as_str()) && !alone.contains(&t.path))
            .collect();
        out.extend(naming(&planned.config, &suites.modules, &step.id, &tests));
    }
    out
}

fn naming(
    config: &Config,
    modules: &Modules,
    runner: &str,
    tests: &[&TestSelection],
) -> Vec<Invocation> {
    match config.tests.runners.items().iter().find(|r| r.id == runner) {
        Some(runner) => invoke::naming(modules, runner, tests),
        None => Vec::new(),
    }
}

/// What a cut needs beside the steps.
pub struct Inputs<'a> {
    pub planned: &'a Planned,
    pub suites: &'a Suites,
    pub classes: &'a BTreeMap<String, Class>,
    /// Seconds for one test file, by runner.
    pub per_file: &'a BTreeMap<String, f64>,
    pub history: &'a History,
    pub budget: u32,
}

/// The steps with each runner cut to what fits the budget, nearest tests
/// first, and what was left as debts. A changed test always runs. A runner
/// with no recorded time runs its nearest `TRIAL` files, which gives the
/// next run its time. A suite reached only through a blanket runs whole
/// when all of it fits what's left, and otherwise only its smoke set.
pub fn cut(inputs: &Inputs, steps: Vec<Invocation>) -> Cut {
    let Inputs {
        planned,
        per_file,
        budget,
        ..
    } = *inputs;
    let budget_s = budget as f64;
    let mut spent: f64 = steps
        .iter()
        .filter(|s| !budgeted(s))
        .map(|s| {
            let times = inputs.history.seconds.get(&s.id).cloned();
            times.map_or(0.0, median)
        })
        .sum();
    let order: Vec<&str> = planned
        .config
        .tests
        .runners
        .items()
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    let mut tests: Vec<(Class, usize, String, &str)> = Vec::new();
    for step in steps.iter().filter(|s| budgeted(s)) {
        let index = order.iter().position(|id| *id == step.id).unwrap_or(0);
        for path in tests_of(planned, step) {
            let class = inputs.classes.get(&path).copied().unwrap_or(Class::Blanket);
            tests.push((class, index, path, order.get(index).copied().unwrap_or("")));
        }
    }
    tests.sort();
    tests.dedup();
    let mut kept: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut owed: BTreeMap<(&str, &'static str), usize> = BTreeMap::new();
    let mut blanket: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut trial: BTreeMap<&str, usize> = BTreeMap::new();
    for (class, _, path, runner) in tests {
        let time = per_file.get(runner).copied();
        let verdict = match (class, time) {
            (Class::Changed, _) => Ok(()),
            (Class::Blanket, Some(_)) => {
                blanket.entry(runner).or_default().push(path);
                continue;
            }
            (Class::Blanket, None) => Err("blanket"),
            (_, Some(t)) if spent + t <= budget_s => Ok(()),
            (_, Some(_)) => Err("budget"),
            (_, None) => {
                let tried = trial.entry(runner).or_insert(0);
                *tried += 1;
                if *tried <= TRIAL {
                    Ok(())
                } else {
                    Err("untimed")
                }
            }
        };
        match verdict {
            Ok(()) => {
                spent += time.unwrap_or(0.0);
                kept.entry(runner).or_default().push(path);
            }
            Err(why) => *owed.entry((runner, why)).or_insert(0) += 1,
        }
    }
    for (runner, paths) in blanket {
        let all = per_file.get(runner).copied().unwrap_or(0.0) * paths.len() as f64;
        if spent + all <= budget_s {
            spent += all;
            kept.entry(runner).or_default().extend(paths);
        } else {
            *owed.entry((runner, "blanket")).or_insert(0) += paths.len();
        }
    }
    let due = match planned.config.phase {
        Some(Phase::Early) => "nightly",
        _ => "merge",
    };
    let ran = kept.values().map(Vec::len).sum();
    Cut {
        steps: rebuilt(inputs, steps, &kept),
        lines: lines(&kept, &owed, due, budget),
        debts: owed
            .iter()
            .map(|((runner, why), files)| Debt {
                step: runner.to_string(),
                files: *files,
                why: why.to_string(),
                due: due.to_string(),
            })
            .collect(),
        ran,
    }
}

/// The steps with each runner's steps replaced, where its first stood, by
/// ones naming the files it keeps, nearest first.
fn rebuilt(
    inputs: &Inputs,
    steps: Vec<Invocation>,
    kept: &BTreeMap<&str, Vec<String>>,
) -> Vec<Invocation> {
    let planned = inputs.planned;
    let by_path: BTreeMap<&str, &TestSelection> = planned
        .plan
        .tests
        .iter()
        .map(|t| (t.path.as_str(), t))
        .collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for step in steps {
        if !budgeted(&step) {
            out.push(step);
            continue;
        }
        if !seen.insert(step.id.clone()) {
            continue;
        }
        let tests: Vec<&TestSelection> = kept
            .get(step.id.as_str())
            .into_iter()
            .flatten()
            .filter_map(|p| by_path.get(p.as_str()).copied())
            .collect();
        out.extend(naming(
            &planned.config,
            &inputs.suites.modules,
            &step.id,
            &tests,
        ));
    }
    out
}

fn when(due: &str) -> &'static str {
    match due {
        "nightly" => "by the nightly run",
        _ => "before merge",
    }
}

fn lines(
    kept: &BTreeMap<&str, Vec<String>>,
    owed: &BTreeMap<(&str, &'static str), usize>,
    due: &str,
    budget: u32,
) -> Vec<String> {
    let mut runners: Vec<&str> = owed.keys().map(|(r, _)| *r).collect();
    runners.dedup();
    runners
        .into_iter()
        .map(|runner| {
            let here = kept.get(runner).map_or(0, Vec::len);
            let parts: Vec<String> = owed
                .iter()
                .filter(|((r, _), _)| *r == runner)
                .map(|((_, why), n)| match *why {
                    "budget" => format!("{n} past the {budget} s budget"),
                    "untimed" => format!(
                        "{n} past its first {TRIAL}, since this runner has no recorded time yet; the next run fits the budget from this one's"
                    ),
                    _ => format!(
                        "{n} reached only through a blanket (a `run_all` path, an unreached file or a wide `[[graph.edges]]` rule)"
                    ),
                })
                .collect();
            let total: usize = owed
                .iter()
                .filter(|((r, _), _)| *r == runner)
                .map(|(_, n)| n)
                .sum();
            format!(
                "{runner}: {here} of {} reached test files run here; {total} owed to CI {}: {}",
                here + total,
                when(due),
                parts.join("; ")
            )
        })
        .collect()
}

/// The pass line's account of what is owed: "; owed to CI before merge:
/// api 251, e2e 119 test files (`fairlead receipt` lists them)".
pub fn owed_line(debts: &[Debt]) -> String {
    if debts.is_empty() {
        return String::new();
    }
    let mut by_step: BTreeMap<&str, usize> = BTreeMap::new();
    for d in debts {
        *by_step.entry(d.step.as_str()).or_insert(0) += d.files;
    }
    let parts: Vec<String> = by_step.iter().map(|(s, n)| format!("{s} {n}")).collect();
    format!(
        "; owed to CI {}: {} test files (`fairlead receipt` lists them)",
        when(&debts[0].due),
        parts.join(", ")
    )
}

/// The receipt's lines for what is owed, one for each runner: "unit: 251
/// test files to CI before merge (240 budget, 11 blanket)".
pub fn owed_lines(debts: &[Debt]) -> Vec<String> {
    let mut steps: Vec<&str> = debts.iter().map(|d| d.step.as_str()).collect();
    steps.dedup();
    steps
        .into_iter()
        .map(|step| {
            let mine: Vec<&Debt> = debts.iter().filter(|d| d.step == step).collect();
            let total: usize = mine.iter().map(|d| d.files).sum();
            let parts: Vec<String> = mine
                .iter()
                .map(|d| format!("{} {}", d.files, d.why))
                .collect();
            format!(
                "{step}: {total} test files to CI {} ({})",
                when(&mine[0].due),
                parts.join(", ")
            )
        })
        .collect()
}

/// Advice the budget's reading of the plan backs: a suite a blanket
/// reaches with no smoke set, and a new source file no test is near.
pub fn advice(
    planned: &Planned,
    suites: &Suites,
    classes: &BTreeMap<String, Class>,
) -> Vec<String> {
    let mut out = Vec::new();
    for runner in planned.config.tests.runners.items() {
        let mine = |t: &&TestSelection| t.runner.as_deref() == Some(runner.id.as_str());
        let blanket = planned
            .plan
            .tests
            .iter()
            .filter(mine)
            .filter(|t| classes.get(&t.path) == Some(&Class::Blanket))
            .count();
        if blanket > 0 && runner.smoke.is_empty() {
            out.push(format!(
                "{}: {blanket} test files are reached only through a blanket and the runner names no smoke set. Set `smoke = [...]` on it to the few files that stand for the suite, or narrow the rule (`fairlead test --explain FILE` prints the chain)",
                runner.id
            ));
        }
    }
    let lonely: Vec<&str> = planned
        .plan
        .changed
        .iter()
        .filter(|c| c.status == Status::Added)
        .map(|c| c.path.as_str())
        .filter(|p| !suites.tests.contains(*p) && !tested(planned, suites, p))
        .collect();
    if let Some(first) = lonely.first() {
        let more = match lonely.len() - 1 {
            0 => String::new(),
            n => format!(" (and {n} more new files)"),
        };
        out.push(format!(
            "{first}{more} is new and no test is near it: none imports it, directly or through other files. Add a test beside it, so a change to it is proved in seconds"
        ));
    }
    out
}

/// Whether a test imports `path`, directly or through other files. A
/// `[[graph.edges]]` rule doesn't count: it says a suite depends on a
/// folder, not that a test exercises this file. A path outside the graph
/// has nothing to import, so it counts as tested.
fn tested(planned: &Planned, suites: &Suites, path: &str) -> bool {
    let graph = &planned.scan.graph;
    let Some(start) = graph.id(path) else {
        return true;
    };
    let mut seen = BTreeSet::from([start]);
    let mut queue = vec![start];
    while let Some(file) = queue.pop() {
        for (importer, kind) in graph.importers(file) {
            if kind == Some(EdgeKind::Rule) || !seen.insert(importer) {
                continue;
            }
            if suites.tests.contains(&graph.files[importer as usize]) {
                return true;
            }
            queue.push(importer);
        }
    }
    false
}

/// Advice once the steps ran: a step that alone outlasts the budget.
pub fn slow(steps: &[fairlead_guard::events::Step], budget: u32) -> Vec<String> {
    steps
        .iter()
        .filter(|s| s.seconds > budget as f64)
        .map(|s| {
            format!(
                "{} took {} s, longer than the whole {budget} s budget. `fairlead insights --speed` says where its time goes; a parallel mode or a split is the usual fix",
                s.id, s.seconds
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "done_budget_tests.rs"]
mod tests;
