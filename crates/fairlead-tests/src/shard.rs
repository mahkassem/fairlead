//! Splitting a plan across N jobs. Each runner that names its tests (by
//! `{files}`, `{class}`, `{classes}` or `{packages}`) has its test files
//! divided longest first: the heaviest goes to the lightest shard, ties to
//! the lower shard, files of equal weight in path order, so the same plan
//! and durations always give the same shards. A file's weight is its
//! seconds, the median of the known ones when it has none, or 1 without
//! durations; a `{packages}` runner divides whole directories. Everything
//! else runs whole in the first shard: checks, runners that name nothing,
//! tests that run alone, and, in a plan that runs everything, a runner with
//! an `all_command`, since that command finds tests the runner's `match`
//! may not claim.

use std::collections::BTreeMap;

use fairlead_core::config::{Config, Runner};
use fairlead_core::plan::{
    Invocation, InvocationKind, Plan, Shard, ShardWeight, Shards, TestSelection,
};
use fairlead_lang::Scan;

use crate::durations::Durations;
use crate::invoke::{naming, working_dir};
use crate::modules::Modules;

/// The shards, and how many of the divided files had a duration.
pub struct Split {
    pub shards: Shards,
    pub timed: usize,
    pub files: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    File,
    Package,
}

/// How a runner's tests divide, or `None` when it runs whole.
fn unit(runner: &Runner, all: bool) -> Option<Unit> {
    if all && runner.all_command.is_some() {
        return None;
    }
    let command = &runner.command;
    if command.iter().any(|a| a == "{packages}") {
        return Some(Unit::Package);
    }
    command
        .iter()
        .any(|a| a == "{files}" || a.contains("{class}") || a.contains("{classes}"))
        .then_some(Unit::File)
}

/// The file, or for a `{packages}` runner the directory holding it.
fn unit_key(runner: &Runner, all: bool, path: &str) -> String {
    match unit(runner, all) {
        Some(Unit::Package) => path.rsplit_once('/').map_or("", |p| p.0).to_string(),
        _ => path.to_string(),
    }
}

fn median(mut known: Vec<f64>) -> Option<f64> {
    if known.is_empty() {
        return None;
    }
    known.sort_by(f64::total_cmp);
    let mid = known.len() / 2;
    Some(if known.len().is_multiple_of(2) {
        (known[mid - 1] + known[mid]) / 2.0
    } else {
        known[mid]
    })
}

/// Longest processing time first: each weight, heaviest first and then by
/// its position, to the shard with the least so far, the lowest on a tie.
pub fn balance(weights: &[f64], count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by(|a, b| weights[*b].total_cmp(&weights[*a]).then(a.cmp(b)));
    let mut loads = vec![0.0f64; count];
    let mut to = vec![0; weights.len()];
    for i in order {
        let mut best = 0;
        for (k, load) in loads.iter().enumerate() {
            if *load < loads[best] {
                best = k;
            }
        }
        loads[best] += weights[i];
        to[i] = best;
    }
    to
}

struct Divided<'a> {
    runner: &'a Runner,
    tests: Vec<&'a TestSelection>,
}

/// The runners whose files divide, in config order, with their files.
fn divided<'a>(plan: &'a Plan, config: &'a Config) -> Vec<Divided<'a>> {
    let held = |t: &TestSelection| {
        plan.quarantined
            .iter()
            .any(|q| q.kind == InvocationKind::Runner && q.target == t.path)
    };
    let runs = |id: &str| {
        plan.invocations
            .iter()
            .any(|i| i.kind == InvocationKind::Runner && i.id == id && i.quarantined.is_none())
    };
    config
        .tests
        .runners
        .items()
        .iter()
        .filter(|r| unit(r, plan.all).is_some() && runs(&r.id))
        .map(|runner| Divided {
            runner,
            tests: plan
                .tests
                .iter()
                .filter(|t| t.runner.as_deref() == Some(runner.id.as_str()) && !held(t))
                .collect(),
        })
        .filter(|d| !d.tests.is_empty())
        .collect()
}

pub fn split(
    plan: &Plan,
    scan: &Scan,
    config: &Config,
    count: u32,
    durations: Option<&Durations>,
) -> Result<Split, String> {
    if count == 0 {
        return Err("--shards must be 1 or more".into());
    }
    let modules = Modules::discover(&scan.tree, &scan.packages, &config.modules)?;
    let runners = divided(plan, config);
    // Each divided file with its runner's index and its seconds, if known.
    let mut files: Vec<(usize, &TestSelection, Option<f64>)> = Vec::new();
    for (r, d) in runners.iter().enumerate() {
        for t in &d.tests {
            let cwd = working_dir(&modules, d.runner, t);
            files.push((r, t, durations.and_then(|x| x.get(&t.path, &cwd))));
        }
    }
    let timed = files.iter().filter(|f| f.2.is_some()).count();
    let fallback = median(files.iter().filter_map(|f| f.2).collect()).unwrap_or(1.0);
    // Units: a file, or a `{packages}` runner's directory, keyed in path order.
    let mut units: BTreeMap<(usize, String), f64> = BTreeMap::new();
    for (r, t, seconds) in &files {
        let key = unit_key(runners[*r].runner, plan.all, &t.path);
        let weight = if durations.is_some() {
            seconds.unwrap_or(fallback)
        } else {
            1.0
        };
        *units.entry((*r, key)).or_default() += weight;
    }
    let keys: Vec<&(usize, String)> = units.keys().collect();
    let weights: Vec<f64> = units.values().copied().collect();
    let to = balance(&weights, count as usize);
    let shard_of: BTreeMap<&(usize, String), usize> = keys.into_iter().zip(to).collect();
    let mut lists = Vec::new();
    for k in 0..count as usize {
        let mine = |r: usize, t: &TestSelection| {
            shard_of[&(r, unit_key(runners[r].runner, plan.all, &t.path))] == k
        };
        let weight: f64 = units
            .iter()
            .filter(|(key, _)| shard_of[key] == k)
            .map(|(_, w)| w)
            .sum();
        let picked: Vec<Vec<&TestSelection>> = runners
            .iter()
            .enumerate()
            .map(|(r, d)| d.tests.iter().copied().filter(|t| mine(r, t)).collect())
            .collect();
        lists.push(Shard {
            files: picked.iter().flatten().map(|t| t.path.clone()).collect(),
            weight: (weight * 10.0).round() / 10.0,
            invocations: invocations(plan, &modules, &runners, &picked, k == 0),
        });
    }
    let by = match durations {
        Some(_) => ShardWeight::Durations,
        None => ShardWeight::Files,
    };
    Ok(Split {
        shards: Shards {
            count,
            by,
            split: runners.iter().map(|d| d.runner.id.clone()).collect(),
            lists,
        },
        timed,
        files: files.len(),
    })
}

/// One shard's invocations in the plan's order: a divided runner's own
/// files where its invocations stood, and, in the first shard, the rest.
fn invocations(
    plan: &Plan,
    modules: &Modules,
    runners: &[Divided],
    picked: &[Vec<&TestSelection>],
    first: bool,
) -> Vec<Invocation> {
    let mut out = Vec::new();
    let mut placed = vec![false; runners.len()];
    for inv in &plan.invocations {
        let divided = runners.iter().position(|d| {
            inv.kind == InvocationKind::Runner && inv.quarantined.is_none() && d.runner.id == inv.id
        });
        match divided {
            Some(r) if !placed[r] => {
                placed[r] = true;
                out.extend(naming(modules, runners[r].runner, &picked[r]));
            }
            Some(_) => {}
            None if first => out.push(inv.clone()),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heaviest_goes_to_the_lightest_shard_and_a_tie_to_the_lower() {
        assert_eq!(balance(&[1.0, 5.0, 3.0, 3.0, 2.0], 2), vec![1, 0, 1, 1, 0]);
        assert_eq!(balance(&[1.0, 1.0, 1.0, 1.0, 1.0], 3), vec![0, 1, 2, 0, 1]);
        assert_eq!(balance(&[4.0], 3), vec![0]);
    }

    #[test]
    fn the_median_of_an_even_count_is_the_mean_of_the_middle_two() {
        assert_eq!(median(vec![4.0, 1.0, 10.0]), Some(4.0));
        assert_eq!(median(vec![4.0, 1.0, 10.0, 2.0]), Some(3.0));
        assert_eq!(median(Vec::new()), None);
    }
}
