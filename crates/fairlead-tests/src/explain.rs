//! `test --explain`: why a file or check is in the plan, or why it isn't.

use std::collections::{HashMap, VecDeque};

use fairlead_core::config::{Config, TestClass};
use fairlead_core::plan::{Plan, Reason};
use fairlead_lang::Scan;

use crate::modules::Modules;
use crate::render;
use crate::testfiles::discover;

fn chain(reason: &Reason) -> String {
    match reason {
        Reason::Import { chain } => {
            let mut lines = vec![chain[0].clone()];
            lines.extend(chain[1..].iter().map(|f| format!("  -> {f}")));
            lines.join("\n")
        }
        other => render::reason(other),
    }
}

/// Every file `id` depends on, directly or not, with the file each was
/// first reached from.
fn dependencies(scan: &Scan, id: u32) -> HashMap<u32, u32> {
    let mut seen = HashMap::from([(id, id)]);
    let mut queue = VecDeque::from([id]);
    while let Some(file) = queue.pop_front() {
        for (to, _) in scan.graph.dependencies(file) {
            if !seen.contains_key(to) {
                seen.insert(*to, file);
                queue.push_back(*to);
            }
        }
    }
    seen
}

/// The first barrier on the way from `changed` to `test`, other than `test`:
/// the file where the planner's walk, which starts at the change, stops.
fn barrier_between(scan: &Scan, deps: &HashMap<u32, u32>, test: u32, changed: u32) -> Option<u32> {
    let mut path = vec![changed];
    let mut at = changed;
    while at != test {
        at = *deps.get(&at)?;
        path.push(at);
    }
    path.pop();
    path.into_iter().find(|&f| scan.graph.is_barrier(f))
}

pub fn explain(plan: &Plan, scan: &Scan, config: &Config, target: &str) -> Result<String, String> {
    if let Some(test) = plan.tests.iter().find(|t| t.path == target) {
        return Ok(format!(
            "{target} is selected ({}):\n{}",
            class_name(test.class),
            chain(&test.reason)
        ));
    }
    if let Some(check) = plan.checks.iter().find(|c| c.id == target) {
        return Ok(format!(
            "check {target} is selected: {}",
            render::reason(&check.reason)
        ));
    }
    if config.checks.items().iter().any(|c| c.id == target) {
        return Ok(format!("check {target} isn't selected: no changed file matches its paths and it watches no affected module"));
    }
    let modules = Modules::discover(&scan.tree, &scan.packages, &config.modules)?;
    let found = discover(&scan.tree, config, &modules)?;
    let Some(test) = found.tests.iter().find(|t| t.path == target) else {
        return Err(format!(
            "{target} isn't a test file (tests.match) or a check id"
        ));
    };
    let why = match test.class {
        TestClass::Demand => "it's a demand test, which runs only when it changes".to_string(),
        TestClass::Own => {
            "it's an own test, which runs only when it changes or an owner rule claims it"
                .to_string()
        }
        _ => {
            let id = scan.graph.id(target);
            let deps = id.map(|id| dependencies(scan, id)).unwrap_or_default();
            let stopped = id.and_then(|id| {
                plan.changed.iter().find_map(|c| {
                    let c = scan.graph.id(&c.path).filter(|c| deps.contains_key(c))?;
                    let b = barrier_between(scan, &deps, id, c)?;
                    Some((
                        scan.graph.files[c as usize].clone(),
                        scan.graph.files[b as usize].clone(),
                    ))
                })
            });
            match stopped {
                Some((changed, barrier)) => format!(
                    "it depends on {changed}, but only through {barrier}, a graph.barrier file the walk doesn't go past"
                ),
                None => format!(
                    "none of the {} changed files is among the {} files it depends on",
                    plan.changed.len(),
                    deps.len().saturating_sub(1)
                ),
            }
        }
    };
    Ok(format!("{target} isn't selected: {why}"))
}

fn class_name(class: TestClass) -> &'static str {
    match class {
        TestClass::Unit => "unit",
        TestClass::Own => "own",
        TestClass::Demand => "demand",
        TestClass::Canary => "canary",
    }
}
