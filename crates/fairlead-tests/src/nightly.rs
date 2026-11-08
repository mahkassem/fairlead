//! Whole suites left to a later stage. A runner or check whose `all_from`
//! (set, or by the project's phase) is later than the plan's stage never
//! runs every test or every file it matches there: it runs what the change
//! reaches, and the rest waits for that stage or a local `--full`.

use std::collections::BTreeSet;

use fairlead_core::config::CiStage;
use fairlead_core::plan::{
    CheckSelection, DeferWhy, Deferred, InvocationKind, Reason, TestSelection,
};

use fairlead_lang::{EdgeKind, Graph};

use crate::planner::Context;
use crate::select::{runs_by_default, select};
use crate::walk::Walk;

/// The plan's tests and checks with whole suites taken out.
pub struct Narrowed {
    pub tests: Vec<TestSelection>,
    pub checks: Vec<CheckSelection>,
    /// Runners and checks that name their files even in a plan of everything.
    pub named: BTreeSet<String>,
    pub deferred: Vec<Deferred>,
}

impl Narrowed {
    pub fn unchanged(tests: Vec<TestSelection>, checks: Vec<CheckSelection>) -> Narrowed {
        Narrowed {
            tests,
            checks,
            named: BTreeSet::new(),
            deferred: Vec::new(),
        }
    }
}

/// What the plan is for, as the narrowing needs it.
pub struct Scope<'a> {
    /// The stage the plan is made for; a local plan counts as `ready`.
    pub stage: CiStage,
    /// Why the plan runs everything, if it does.
    pub all_reason: Option<&'a Reason>,
    /// Whether that came from `plan.run_all`, before any narrowed selection.
    pub triggered: bool,
}

fn whole(
    id: &str,
    kind: InvocationKind,
    from: CiStage,
    selected: usize,
    reached: usize,
) -> Deferred {
    Deferred {
        id: id.to_string(),
        kind,
        from,
        selected,
        why: DeferWhy::Whole,
        reached,
        by_rule: 0,
    }
}

/// Whether a test's chain from the change takes a `[[graph.edges]]` hop.
fn through_rule(graph: &Graph, reason: &Reason) -> bool {
    let Reason::Import { chain } = reason else {
        return false;
    };
    chain.windows(2).any(|pair| {
        let (Some(from), Some(to)) = (graph.id(&pair[0]), graph.id(&pair[1])) else {
            return false;
        };
        graph
            .importers(from)
            .into_iter()
            .find(|(i, _)| *i == to)
            .is_some_and(|(_, kind)| kind == Some(EdgeKind::Rule))
    })
}

pub fn apply(
    cx: &Context,
    walked: &Walk,
    tests: Vec<TestSelection>,
    checks: Vec<CheckSelection>,
    scope: &Scope,
) -> Result<Narrowed, String> {
    let config = cx.config;
    let later = |set: Option<CiStage>| config.all_from_of(set).filter(|s| *s > scope.stage);
    let mut out = Narrowed::unchanged(tests, Vec::new());
    // What the change reaches, as a plan without `run_all` would pick it; a
    // test picked only by the widening to everything isn't reached.
    let (narrow, wide) = if scope.triggered
        && config
            .tests
            .runners
            .items()
            .iter()
            .any(|r| later(r.all_from).is_some())
    {
        let (t, _, a) = select(cx, walked, &mut Vec::new())?;
        (t, a)
    } else {
        (out.tests.clone(), scope.all_reason.cloned())
    };
    let mut moved = false;
    for (index, runner) in config.tests.runners.items().iter().enumerate() {
        let Some(from) = later(runner.all_from) else {
            continue;
        };
        let id = runner.id.as_str();
        let mine = |t: &TestSelection| t.runner.as_deref() == Some(id);
        if !out.tests.iter().any(&mine) {
            continue;
        }
        let suite = cx
            .tests
            .iter()
            .filter(|t| t.runner == Some(index) && runs_by_default(cx, t))
            .count();
        let reached: Vec<TestSelection> = narrow
            .iter()
            .filter(|t| mine(t) && Some(&t.reason) != wide.as_ref())
            .cloned()
            .collect();
        let most = reached.len() as f64 >= config.plan.whole_share * suite as f64;
        if scope.all_reason.is_none() && !most {
            continue;
        }
        out.tests.retain(|t| !mine(t));
        moved = true;
        if most && !reached.is_empty() {
            let mut deferred = whole(id, InvocationKind::Runner, from, suite, reached.len());
            let graph = &cx.scan.graph;
            deferred.by_rule = reached
                .iter()
                .filter(|t| through_rule(graph, &t.reason))
                .count();
            out.deferred.push(deferred);
            continue;
        }
        let left = suite.saturating_sub(reached.len());
        out.deferred
            .push(whole(id, InvocationKind::Runner, from, left, 0));
        out.tests.extend(reached);
        out.named.insert(id.to_string());
    }
    if moved {
        out.tests.sort_by(|a, b| a.path.cmp(&b.path));
    }
    for selected in checks {
        let check = config.checks.items().iter().find(|c| c.id == selected.id);
        let Some((check, from)) = check.and_then(|c| later(c.all_from).map(|f| (c, f))) else {
            out.checks.push(selected);
            continue;
        };
        if matches!(
            selected.reason,
            Reason::Paths { .. } | Reason::Modules { .. }
        ) {
            if scope.all_reason.is_some() {
                out.named.insert(check.id.clone());
            }
            out.checks.push(selected);
            continue;
        }
        let paths: Vec<crate::pattern::Pattern> = check
            .paths
            .iter()
            .map(|g| crate::pattern::Pattern::new(g))
            .collect::<Result<_, _>>()?;
        let matched = cx
            .changed_or_scoped()
            .filter(|p| paths.iter().any(|g| g.is_match(p)))
            .count();
        if matched > 0 {
            out.named.insert(check.id.clone());
            out.checks.push(CheckSelection {
                id: selected.id,
                reason: Reason::Paths { matched },
            });
        } else {
            out.deferred
                .push(whole(&check.id, InvocationKind::Check, from, 1, 0));
        }
    }
    Ok(out)
}
