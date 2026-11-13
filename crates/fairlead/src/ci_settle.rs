//! `fairlead ci settle`: the merge check. Every invocation the commit's plan
//! holds needs a green run on that commit, from a results file `ci run
//! --results` wrote or from a CI job the config names for its step. What has
//! neither is unsettled, and the check fails naming its test files.

use std::collections::{BTreeMap, BTreeSet};

use fairlead_core::config::CiStage;
use fairlead_core::plan::{Invocation, InvocationKind, Plan};
use serde::{Deserialize, Serialize};

use crate::ci_report::{Ran, Results};

/// One thing the plan has to see run: an invocation, or one shard's share of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub id: String,
    pub kind: InvocationKind,
    /// The shard that holds it, numbered from 1, in a sharded plan.
    pub shard: Option<u32>,
    pub cwd: String,
    pub argv: Vec<String>,
    /// The test files it covers; none for a check.
    pub files: Vec<String>,
}

/// What a named CI job concluded on the commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    /// `success`, `failure`, `skipped`, `cancelled`, or `pending` and `absent`.
    pub conclusion: String,
}

/// Why a unit counts as run and green.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "kebab-case")]
pub enum Proof {
    /// A results file for this plan holds it, passed.
    Results,
    /// The config names this job for the step, and it concluded `success`.
    Job { name: String },
}

/// A step's units that are settled, as the record lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settled {
    pub id: String,
    pub files: usize,
    #[serde(flatten)]
    pub proof: Proof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unsettled {
    pub id: String,
    pub kind: InvocationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard: Option<u32>,
    /// Why: nothing ran it, it failed, or the only run was a held failure.
    pub why: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

/// A step this stage leaves for a later one: a debt not due yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    pub id: String,
    /// `ready`, `merge` or `nightly`: the run that has to collect it.
    pub due: String,
    pub files: usize,
}

/// What one run of the check found, as `--out` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub version: u32,
    pub plan_id: String,
    pub head: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub settled: Vec<Settled>,
    pub unsettled: Vec<Unsettled>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<Waiting>,
}

impl Record {
    pub fn passed(&self) -> bool {
        self.unsettled.is_empty()
    }
}

/// The test files each runner's selection holds.
fn files_by_runner(plan: &Plan) -> BTreeMap<&str, Vec<String>> {
    let mut by: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for t in &plan.tests {
        if let Some(runner) = t.runner.as_deref() {
            by.entry(runner).or_default().push(t.path.clone());
        }
    }
    by
}

fn unit(
    inv: &Invocation,
    shard: Option<u32>,
    files: &BTreeMap<&str, Vec<String>>,
    only: Option<&BTreeSet<&str>>,
) -> Unit {
    let mut covered = match inv.kind {
        InvocationKind::Runner => files.get(inv.id.as_str()).cloned().unwrap_or_default(),
        InvocationKind::Check => Vec::new(),
    };
    // A split runner's shard covers only the files that shard was given.
    if let Some(only) = only {
        covered.retain(|f| only.contains(f.as_str()));
    }
    Unit {
        id: inv.id.clone(),
        kind: inv.kind.clone(),
        shard,
        cwd: inv.cwd.clone(),
        argv: inv.argv.clone(),
        files: covered,
    }
}

/// Everything the plan has to see run: its invocations, or for a sharded
/// plan each shard's.
pub fn units(plan: &Plan) -> Vec<Unit> {
    let files = files_by_runner(plan);
    let Some(shards) = plan.shards.as_ref().filter(|s| s.count > 1) else {
        return (plan.invocations.iter())
            .map(|i| unit(i, None, &files, None))
            .collect();
    };
    let mut out = Vec::new();
    for (at, shard) in shards.lists.iter().enumerate() {
        let given: BTreeSet<&str> = shard.files.iter().map(String::as_str).collect();
        for inv in &shard.invocations {
            let split = shards.split.contains(&inv.id);
            out.push(unit(
                inv,
                Some(at as u32 + 1),
                &files,
                split.then_some(&given),
            ));
        }
    }
    out
}

fn same(ran: &Ran, id: &str, cwd: &str, argv: &[String]) -> bool {
    ran.id == id && ran.cwd == cwd && ran.argv == argv
}

/// The runs of one invocation, from the newest attempt that ran it: a
/// re-run's outcome replaces the run it repeats.
fn newest<'a>(results: &'a [Results], id: &str, cwd: &str, argv: &[String]) -> Vec<&'a Ran> {
    let runs: Vec<(u32, &Ran)> = (results.iter())
        .flat_map(|r| {
            let attempt = r.attempt.unwrap_or(0);
            r.invocations.iter().map(move |ran| (attempt, ran))
        })
        .filter(|(_, ran)| same(ran, id, cwd, argv))
        .collect();
    let last = runs.iter().map(|(a, _)| *a).max().unwrap_or(0);
    (runs.into_iter())
        .filter(|(a, _)| *a == last)
        .map(|(_, ran)| ran)
        .collect()
}

fn passed(results: &[Results], id: &str, cwd: &str, argv: &[String]) -> bool {
    newest(results, id, cwd, argv).iter().any(|r| r.passed)
}

/// Why a unit with no green run is unsettled, from the runs that named it.
fn why(unit: &Unit, results: &[Results], jobs: &[Job]) -> String {
    let runs = newest(results, &unit.id, &unit.cwd, &unit.argv);
    let named: Vec<String> = (jobs.iter())
        .map(|j| format!("job `{}` is {}", j.name, j.conclusion))
        .collect();
    let ran = match runs.len() {
        0 => "no job ran it".to_string(),
        _ if runs.iter().all(|r| r.quarantined) => {
            "it only failed as quarantined; it needs a green run on this commit".to_string()
        }
        _ => "it failed".to_string(),
    };
    match named.is_empty() {
        true => ran,
        false => format!("{ran}, and {}", named.join(", ")),
    }
}

/// Judges the plan against the results for it and the named jobs' conclusions.
/// `jobs` maps a step id to the jobs the config names for it; results for
/// another plan are left out.
pub fn settle(plan: &Plan, results: &[Results], jobs: &BTreeMap<String, Vec<Job>>) -> Record {
    let results: Vec<Results> = (results.iter())
        .filter(|r| r.plan_id == plan.plan_id)
        .cloned()
        .collect();
    // A run of the plan's unsplit invocation covers every shard's share of it.
    let whole: BTreeSet<(&str, &str)> = (plan.invocations.iter())
        .filter(|i| passed(&results, &i.id, &i.cwd, &i.argv))
        .map(|i| (i.id.as_str(), i.cwd.as_str()))
        .collect();
    let mut settled: BTreeMap<(String, Proof), usize> = BTreeMap::new();
    let mut unsettled = Vec::new();
    for unit in units(plan) {
        let named = jobs.get(&unit.id).map(Vec::as_slice).unwrap_or_default();
        let by_results = passed(&results, &unit.id, &unit.cwd, &unit.argv)
            || (unit.shard.is_some() && whole.contains(&(unit.id.as_str(), unit.cwd.as_str())));
        // Every job named for the step has to be green: one of two is half a proof.
        let attested = !named.is_empty() && named.iter().all(|j| j.conclusion == "success");
        let proof = match (by_results, attested) {
            (true, _) => Some(Proof::Results),
            (false, true) => Some(Proof::Job {
                name: (named.iter().map(|j| j.name.as_str()))
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
            (false, false) => None,
        };
        match proof {
            Some(proof) => {
                *settled.entry((unit.id.clone(), proof)).or_default() += unit.files.len();
            }
            None => unsettled.push(Unsettled {
                why: why(&unit, &results, named),
                id: unit.id,
                kind: unit.kind,
                shard: unit.shard,
                files: unit.files,
            }),
        }
    }
    let waiting = (plan.deferred.iter())
        .map(|d| Waiting {
            id: d.id.clone(),
            due: match d.from {
                CiStage::Full => "nightly".to_string(),
                CiStage::Ready => "ready".to_string(),
                _ => "merge".to_string(),
            },
            files: match d.kind {
                InvocationKind::Runner => d.selected,
                InvocationKind::Check => 0,
            },
        })
        .collect();
    Record {
        version: 1,
        plan_id: plan.plan_id.clone(),
        head: plan.head.clone(),
        stage: plan.stage.map(|s| s.to_string()),
        settled: (settled.into_iter())
            .map(|((id, proof), files)| Settled { id, files, proof })
            .collect(),
        unsettled,
        waiting,
    }
}
