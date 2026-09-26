//! Building a plan, in the order the design sets: changed paths, `run_all`,
//! deleted files, package manifests, the reverse walk, tests, unreached
//! files, checks, then the invocations that run them.

use std::collections::BTreeSet;

use fairlead_core::config::{Config, Unresolved};
use fairlead_core::plan::{Change, Plan, Reason, Status, Warning, VERSION};
use fairlead_lang::deleted::attach_deleted;
use fairlead_lang::tree::{parent, Tree};
use fairlead_lang::Scan;

use crate::checks::checks;
use crate::invoke::invocations;
use crate::modules::Modules;
use crate::owners::Owners;
use crate::pattern::Pattern;
use crate::select::{everything, select};
use crate::testfiles::{discover, TestFile};
use crate::walk::{walk, Via, Walk};

pub struct Input {
    pub changes: Vec<Change>,
    pub base: Option<String>,
    pub head: String,
    pub config_digest: String,
    pub tree_hash: String,
}

/// Everything the selection steps share.
pub struct Context<'a> {
    pub scan: &'a Scan,
    pub config: &'a Config,
    pub modules: Modules,
    pub owners: Owners,
    pub tests: Vec<TestFile>,
    /// Changed paths, a rename's old path included, ignored ones too.
    pub changed: BTreeSet<String>,
    pub deleted: BTreeSet<String>,
    /// Changed paths `plan.ignore` matched that nothing references.
    pub ignored: BTreeSet<String>,
}

pub fn patterns(globs: &[String]) -> Result<Vec<Pattern>, String> {
    globs.iter().map(|g| Pattern::new(g)).collect()
}

fn changed_sets(changes: &[Change]) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut changed = BTreeSet::new();
    let mut deleted = BTreeSet::new();
    for change in changes {
        changed.insert(change.path.clone());
        if change.status == Status::Deleted {
            deleted.insert(change.path.clone());
        }
        if let Some(from) = &change.from {
            changed.insert(from.clone());
            deleted.insert(from.clone());
        }
    }
    (changed, deleted)
}

fn check_unresolved(scan: &Scan, config: &Config) -> Result<(), String> {
    if config.graph.unresolved != Unresolved::Fail || scan.graph.unresolved.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = scan
        .graph
        .unresolved
        .iter()
        .take(20)
        .map(|(from, spec)| format!("  {}: {spec}", scan.graph.files[*from as usize]))
        .collect();
    Err(format!(
        "{} imports don't resolve and graph.unresolved = \"fail\":\n{}",
        scan.graph.unresolved.len(),
        lines.join("\n")
    ))
}

pub fn plan(scan: &mut Scan, config: &Config, input: Input) -> Result<Plan, String> {
    check_unresolved(scan, config)?;
    let (changed, deleted) = changed_sets(&input.changes);
    let phantoms: Vec<String> = deleted.iter().cloned().collect();
    attach_deleted(scan, &config.graph, &phantoms);
    let scan: &Scan = scan;
    let modules = Modules::discover(&scan.tree, &scan.packages, &config.modules)?;
    let found = discover(&scan.tree, config, &modules)?;
    if !found.unmatched.is_empty() || !found.ambiguous.is_empty() {
        return Err(runner_problems(&found.unmatched, &found.ambiguous));
    }
    let ignore = patterns(config.plan.ignore.items())?;
    let ignored = changed
        .iter()
        .filter(|p| !Tree::is_source(p) && ignore.iter().any(|g| g.is_match(p)))
        .filter(|p| {
            scan.graph
                .id(p)
                .is_none_or(|id| scan.graph.importers(id).is_empty())
        })
        .cloned()
        .collect();
    let owners = Owners::new(config.tests.owners.items())?;
    let cx = Context {
        scan,
        config,
        modules,
        owners,
        tests: found.tests,
        changed,
        deleted,
        ignored,
    };
    let run_all = patterns(config.plan.run_all.items())?;
    let trigger = cx
        .changed
        .iter()
        .find(|p| run_all.iter().any(|g| g.is_match(p)))
        .cloned();
    let mut warnings = Vec::new();
    let walked = start_walk(&cx);
    let (tests, unreached, all_reason) = match trigger {
        Some(path) => {
            let reason = Reason::RunAll { path };
            (everything(&cx, &reason), Vec::new(), Some(reason))
        }
        None => select(&cx, &walked, &mut warnings)?,
    };
    let all = all_reason.is_some();
    let checks = checks(&cx, &walked, all_reason.as_ref())?;
    warn_unresolved(&cx, &walked, &mut warnings);
    if !tests.is_empty() && config.tests.runners.items().is_empty() {
        warnings.push(Warning {
            code: "no-runners".into(),
            path: None,
            message: "tests were selected but no [[tests.runners]] is configured to run them"
                .into(),
        });
    }
    let invocations = invocations(&cx, &tests, &checks, all);
    let plan_id = crate::digest::plan_id(
        &input.config_digest,
        &input.tree_hash,
        input.base.as_deref(),
        &input.changes,
    );
    Ok(Plan {
        version: VERSION,
        plan_id,
        fairlead_version: env!("CARGO_PKG_VERSION").to_string(),
        config_digest: input.config_digest,
        tree_hash: input.tree_hash,
        base: input.base,
        head: input.head,
        all,
        changed: input.changes,
        ignored: cx.ignored.iter().cloned().collect(),
        tests,
        checks,
        invocations,
        unreached,
        warnings,
    })
}

/// A workspace package's own `package.json`, which stands for the package.
pub fn is_manifest(cx: &Context, path: &str) -> bool {
    let dir = parent(path);
    path.ends_with("package.json") && cx.scan.packages.iter().any(|p| p.dir == dir)
}

/// The walk from every changed path that isn't ignored, with a changed
/// package manifest standing for every file in its package.
pub fn start_walk(cx: &Context) -> Walk {
    let graph = &cx.scan.graph;
    let mut starts: Vec<(u32, Via)> = Vec::new();
    for path in cx.changed.iter().filter(|p| !cx.ignored.contains(*p)) {
        if let Some(id) = graph.id(path) {
            starts.push((id, Via::Start));
        }
        if is_manifest(cx, path) {
            let dir = parent(path);
            let prefix = format!("{dir}/");
            for (id, file) in graph.files.iter().enumerate() {
                if file.starts_with(&prefix) && file != path {
                    starts.push((id as u32, Via::Path(path.clone())));
                }
            }
        }
    }
    walk(graph, &cx.modules, starts)
}

fn warn_unresolved(cx: &Context, walked: &Walk, warnings: &mut Vec<Warning>) {
    for (from, spec) in &cx.scan.graph.unresolved {
        if walked.reached.contains_key(from) {
            warnings.push(Warning {
                code: "unresolved-import".into(),
                path: Some(cx.scan.graph.files[*from as usize].clone()),
                message: format!("`{spec}` doesn't resolve; its module is treated as a dependency"),
            });
        }
    }
}

fn runner_problems(unmatched: &[String], ambiguous: &[(String, Vec<String>)]) -> String {
    let mut lines = Vec::new();
    if !unmatched.is_empty() {
        lines.push(format!(
            "{} test files match no [[tests.runners]]:",
            unmatched.len()
        ));
        lines.extend(unmatched.iter().take(20).map(|p| format!("  {p}")));
    }
    for (path, ids) in ambiguous.iter().take(20) {
        lines.push(format!(
            "{path} matches more than one runner: {}",
            ids.join(", ")
        ));
    }
    lines.join("\n")
}
