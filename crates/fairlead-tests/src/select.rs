//! Which tests a change selects, and what happens to changed files no test
//! depends on. A test's reason is the first that applies: it changed, it
//! depends on a change, an owner rule claims it, or it's a canary. A changed
//! file with no test in its reverse closure is unreached, and widens by
//! policy unless an owner rule covers it.

use std::collections::{BTreeMap, HashSet};

use fairlead_core::config::{TestClass, Unreached};
use fairlead_core::plan::{Reason, TestSelection, UnreachedFile, Warning};

use crate::planner::Context;
use crate::testfiles::TestFile;
use crate::walk::{walk_until, Via, Walk};

pub fn selection(cx: &Context, test: &TestFile, reason: Reason) -> TestSelection {
    TestSelection {
        path: test.path.clone(),
        module: test.module.map(|m| cx.modules.get(m).name.clone()),
        runner: test
            .runner
            .map(|r| cx.config.tests.runners.items()[r].id.clone()),
        class: test.class,
        reason,
    }
}

fn runs_by_default(cx: &Context, test: &TestFile) -> bool {
    test.class != TestClass::Demand || cx.changed.contains(&test.path)
}

/// Every test but `demand` ones, which run only when named or changed.
pub fn everything(cx: &Context, reason: &Reason) -> Vec<TestSelection> {
    cx.tests
        .iter()
        .filter(|t| runs_by_default(cx, t))
        .map(|t| {
            let reason = if cx.changed.contains(&t.path) {
                Reason::Changed
            } else {
                reason.clone()
            };
            selection(cx, t, reason)
        })
        .collect()
}

fn first_reason(
    cx: &Context,
    walked: &Walk,
    test: &TestFile,
    claim: Option<Reason>,
) -> Option<Reason> {
    if cx.changed.contains(&test.path) && !cx.deleted.contains(&test.path) {
        return Some(Reason::Changed);
    }
    let graph = &cx.scan.graph;
    let reached = graph
        .id(&test.path)
        .filter(|id| walked.reached.contains_key(id));
    match test.class {
        TestClass::Unit => reached
            .map(|id| Reason::Import {
                chain: walked.chain(graph, id),
            })
            .or(claim),
        TestClass::Own => claim,
        TestClass::Canary => Some(Reason::Canary),
        TestClass::Demand => None,
    }
}

pub type Selected = (Vec<TestSelection>, Vec<UnreachedFile>, Option<Reason>);

pub fn select(
    cx: &Context,
    walked: &Walk,
    warnings: &mut Vec<Warning>,
) -> Result<Selected, String> {
    let paths: Vec<&str> = cx.tests.iter().map(|t| t.path.as_str()).collect();
    let claims: BTreeMap<usize, Reason> = cx
        .owners
        .claims(cx.changed.iter().map(String::as_str), &paths)?
        .into_iter()
        .map(|(t, c)| {
            (
                t,
                Reason::Owner {
                    rule: c.rule,
                    covers: c.covers,
                    changed: c.changed,
                },
            )
        })
        .collect();
    let mut chosen: BTreeMap<String, TestSelection> = BTreeMap::new();
    for (i, test) in cx.tests.iter().enumerate() {
        if let Some(reason) = first_reason(cx, walked, test, claims.get(&i).cloned()) {
            chosen.insert(test.path.clone(), selection(cx, test, reason));
        }
    }
    let (unreached, all_reason) = unreached(cx, &mut chosen, warnings);
    if let Some(reason) = &all_reason {
        for test in everything(cx, reason) {
            chosen.entry(test.path.clone()).or_insert(test);
        }
    }
    Ok((chosen.into_values().collect(), unreached, all_reason))
}

/// Whether any test is in `path`'s reverse closure.
fn reaches_a_test(cx: &Context, path: &str, tests: &HashSet<u32>) -> bool {
    let graph = &cx.scan.graph;
    let Some(id) = graph.id(path) else {
        return false;
    };
    walk_until(graph, &cx.modules, vec![(id, Via::Start)], |f| {
        tests.contains(&f)
    })
    .1
}

fn unreached(
    cx: &Context,
    chosen: &mut BTreeMap<String, TestSelection>,
    warnings: &mut Vec<Warning>,
) -> (Vec<UnreachedFile>, Option<Reason>) {
    let graph = &cx.scan.graph;
    let test_ids: HashSet<u32> = cx.tests.iter().filter_map(|t| graph.id(&t.path)).collect();
    let is_test: HashSet<&str> = cx.tests.iter().map(|t| t.path.as_str()).collect();
    let test_globs = crate::planner::patterns(cx.config.tests.matches.items()).unwrap_or_default();
    // A deleted test has nothing left to run, and a manifest already stands for its package.
    let skip = |p: &str| {
        (cx.deleted.contains(p) && test_globs.iter().any(|g| g.is_match(p)))
            || crate::planner::is_manifest(cx, p)
    };
    let mut out = Vec::new();
    let mut all_reason = None;
    for path in &cx.changed {
        if cx.ignored.contains(path)
            || is_test.contains(path.as_str())
            || skip(path)
            || reaches_a_test(cx, path, &test_ids)
        {
            continue;
        }
        let module = cx.modules.of(path);
        let module_name = module.map(|m| cx.modules.get(m).name.clone());
        let module_tests: Vec<&TestFile> = cx
            .tests
            .iter()
            .filter(|t| module.is_some() && t.module == module && runs_by_default(cx, t))
            .collect();
        let policy = cx.config.tests.unreached;
        let selected = if cx.owners.covers(path) {
            "owner"
        } else if policy == Unreached::Warn {
            "none"
        } else if policy == Unreached::All || module_tests.is_empty() {
            "all"
        } else {
            "module"
        };
        match selected {
            "all" if all_reason.is_none() => {
                all_reason = Some(Reason::Unreached {
                    path: path.clone(),
                    policy: "all".into(),
                });
            }
            "module" => {
                for test in module_tests {
                    let reason = Reason::Unreached {
                        path: path.clone(),
                        policy: "module".into(),
                    };
                    chosen
                        .entry(test.path.clone())
                        .or_insert_with(|| selection(cx, test, reason));
                }
            }
            _ => {}
        }
        warnings.push(Warning {
            code: "unreached".into(),
            path: Some(path.clone()),
            message: format!("no test depends on this file; selected: {selected}"),
        });
        out.push(UnreachedFile {
            path: path.clone(),
            module: module_name,
            selected: selected.into(),
        });
    }
    (out, all_reason)
}
