//! `test --explain`: why a file or check is in the plan, or why it isn't.

use std::collections::{HashSet, VecDeque};

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

/// Every file `id` depends on, directly or not.
fn dependencies(scan: &Scan, id: u32) -> HashSet<u32> {
    let mut seen = HashSet::from([id]);
    let mut queue = VecDeque::from([id]);
    while let Some(file) = queue.pop_front() {
        for (to, _) in scan.graph.dependencies(file) {
            if seen.insert(*to) {
                queue.push_back(*to);
            }
        }
    }
    seen
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
            let deps = scan
                .graph
                .id(target)
                .map(|id| dependencies(scan, id))
                .unwrap_or_default();
            format!(
                "none of the {} changed files is among the {} files it depends on",
                plan.changed.len(),
                deps.len().saturating_sub(1)
            )
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
