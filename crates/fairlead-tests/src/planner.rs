//! Building a plan, in the order the design sets: changed paths, the trims,
//! `run_all`, deleted files, package manifests, the reverse walk, tests,
//! unreached files, checks, then the invocations that run them.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fairlead_core::config::{Config, LockfileMode, TestClass, Unresolved};
use fairlead_core::plan::{Change, Plan, Reason, Status, Warning, VERSION};
use fairlead_lang::deleted::attach_deleted;
use fairlead_lang::tree::parent;
use fairlead_lang::Scan;

use crate::checks::checks;
use crate::invoke::invocations;
use crate::modules::Modules;
use crate::owners::Owners;
use crate::pattern::Pattern;
use crate::select::{everything, select};
use crate::testfiles::{discover, TestFile};
use crate::trims::Trims;
use crate::walk::{walk, Via, Walk};

pub struct Input {
    pub changes: Vec<Change>,
    pub base: Option<String>,
    pub head: String,
    pub config_digest: String,
    pub tree_hash: String,
    /// Files' text at the base, for the changes that need it: lockfiles,
    /// manifests and workflows.
    pub base_files: BTreeMap<String, String>,
    /// Run everything whatever changed, named as the reason, such as a
    /// full CI stage.
    pub everything: Option<String>,
}

/// The one lockfile the planner can scope, at the repository root.
pub const LOCKFILE: &str = "pnpm-lock.yaml";

/// The base-side files a plan of `changes` needs, read from git.
pub fn base_files(
    root: &std::path::Path,
    base: &str,
    changes: &[Change],
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let paths = changes
        .iter()
        .flat_map(|c| std::iter::once(&c.path).chain(&c.from))
        .filter(|p| *p == LOCKFILE || crate::trims::reads_base(p));
    for path in paths {
        if let Some(text) = crate::git::file_at(root, base, path) {
            out.insert(path.clone(), text);
        }
    }
    out
}

/// A lockfile change narrowed to the packages it reaches: their manifests
/// stand in for it wherever a changed path would count.
pub struct LockScope {
    pub manifests: BTreeSet<String>,
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
    pub lockfile: Option<LockScope>,
    pub trims: Trims,
}

impl Context<'_> {
    /// Changed paths plus the manifests a scoped lockfile change stands for,
    /// for rules that match paths: owners and checks.
    pub fn changed_or_scoped(&self) -> impl Iterator<Item = &str> + Clone {
        self.changed.iter().map(String::as_str).chain(
            self.lockfile
                .iter()
                .flat_map(|l| l.manifests.iter().map(String::as_str)),
        )
    }
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
    // Refreshing the coverage map changes what the plan knows, not the code.
    let map = config.graph.coverage.as_ref().map(|c| c.map.as_str());
    let is_test = |p: &str| found.tests.iter().any(|t| t.path == p);
    let ignored = changed
        .iter()
        .filter(|p| {
            Some(p.as_str()) == map || (!is_test(p) && ignore.iter().any(|g| g.is_match(p)))
        })
        .cloned()
        .collect();
    let owners = Owners::new(config.tests.owners.items())?;
    let lockfile = lockfile_scope(scan, config, &changed, &input.base_files);
    let run_all = patterns(config.plan.run_all.items())?;
    let mut trims = crate::trims::trims(scan, config, &input.changes, &input.base_files, &run_all);
    let mut warnings = std::mem::take(&mut trims.warnings);
    let mut cx = Context {
        scan,
        config,
        modules,
        owners,
        tests: found.tests,
        changed,
        deleted,
        ignored,
        lockfile,
        trims,
    };
    // An ignored path still selects the tests it reaches; it's only kept from
    // widening the plan when it reaches none.
    let test_ids: HashSet<u32> = cx
        .tests
        .iter()
        .filter_map(|t| scan.graph.id(&t.path))
        .collect();
    cx.ignored = cx
        .ignored
        .iter()
        .filter(|p| !crate::select::reaches_a_test(&cx, p, &test_ids))
        .cloned()
        .collect();
    // Only tests a claim selects count: `demand` never runs on a claim.
    let test_paths: Vec<&str> = cx
        .tests
        .iter()
        .filter(|t| matches!(t.class, TestClass::Unit | TestClass::Own))
        .map(|t| t.path.as_str())
        .collect();
    let mut trigger = None;
    for path in &cx.changed {
        let scoped = cx.lockfile.is_some() && path == LOCKFILE;
        if scoped || cx.trims.quiet.contains(path) || !run_all.iter().any(|g| g.is_match(path)) {
            continue;
        }
        if !cx.owners.overrides_run_all(path, &test_paths)? {
            trigger = Some(path.clone());
            break;
        }
    }
    if cx.lockfile.as_ref().is_some_and(|l| l.manifests.is_empty()) {
        warnings.push(Warning {
            code: "lockfile-scoped-to-nothing".into(),
            path: Some(LOCKFILE.into()),
            message: "the lockfile changed but no workspace package's resolved dependencies did"
                .into(),
        });
    }
    warnings.extend(coverage_warning(scan, config));
    if trigger.is_none() {
        if let Some((path, warning)) = provider_failed(scan, &cx.changed) {
            warnings.push(warning);
            trigger = Some(path);
        }
    }
    if trigger.is_none() {
        trigger = input.everything.clone();
    }
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
    // An empty universe is almost always a config that doesn't fit the
    // repository yet, which reads as a plan that found nothing to run.
    if cx.tests.is_empty() && config.checks.items().is_empty() {
        warnings.push(Warning {
            code: "no-tests".into(),
            path: None,
            message: "no file matches [tests] match, so no test can be selected; `fairlead init` writes a first config for this repository".into(),
        });
    }
    if !tests.is_empty() && config.tests.runners.items().is_empty() {
        warnings.push(Warning {
            code: "no-runners".into(),
            path: None,
            message: "tests were selected but no [[tests.runners]] is configured to run them"
                .into(),
        });
    }
    let quarantined = crate::quarantine::apply(&cx, &tests, &checks, &mut warnings);
    if all {
        warnings.extend(crate::quarantine::narrowed(&cx, &tests, &quarantined));
    }
    let invocations = invocations(&cx, &tests, &checks, all, &quarantined);
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
        quarantined,
        stage: None,
        deferred: Vec::new(),
        reused: None,
    })
}

/// The packages a changed root pnpm lockfile reaches, or `None` to treat it
/// as any other run-all path: scoping is off, there's no base text, pnpm
/// hoists packages where every package can see them, or an affected
/// importer (the root included) isn't a workspace package here.
fn lockfile_scope(
    scan: &Scan,
    config: &Config,
    changed: &BTreeSet<String>,
    base_files: &BTreeMap<String, String>,
) -> Option<LockScope> {
    if config.plan.lockfile != LockfileMode::Scope || !changed.contains(LOCKFILE) {
        return None;
    }
    let base = base_files.get(LOCKFILE)?;
    let head = std::fs::read_to_string(scan.tree.root.join(LOCKFILE)).ok()?;
    if hoists(&scan.tree.root) {
        return None;
    }
    let affected = crate::lockfile::affected_importers(base, &head)?;
    let mut manifests = BTreeSet::new();
    // The root importer (`.`) is no workspace package, so a change there,
    // visible to every package, falls through to run-all here.
    for importer in affected {
        if !scan.packages.iter().any(|p| p.dir == importer) {
            return None;
        }
        manifests.insert(format!("{importer}/package.json"));
    }
    Some(LockScope { manifests })
}

/// Whether pnpm is set to hoist packages to a shared `node_modules`, in
/// `.npmrc` or `pnpm-workspace.yaml`.
fn hoists(root: &std::path::Path) -> bool {
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap_or_default();
    let npmrc = read(".npmrc").to_ascii_lowercase();
    let workspace = read("pnpm-workspace.yaml").to_ascii_lowercase();
    let npmrc_says = npmrc.lines().map(|l| l.replace(' ', "")).any(|l| {
        l.starts_with("node-linker=hoisted")
            || l.starts_with("shamefully-hoist=true")
            || l.starts_with("public-hoist-pattern")
    });
    let workspace_says = workspace.lines().map(|l| l.replace(' ', "")).any(|l| {
        l.starts_with("nodelinker:hoisted")
            || l.starts_with("shamefullyhoist:true")
            || l.starts_with("publichoistpattern")
    });
    npmrc_says || workspace_says
}

/// A workspace package's own `package.json`, which stands for the package.
pub fn is_manifest(cx: &Context, path: &str) -> bool {
    let dir = parent(path);
    path.ends_with("package.json") && cx.scan.packages.iter().any(|p| p.dir == dir)
}

/// The walk from every changed path that isn't ignored, with a changed
/// package manifest standing for every file in its package.
/// A coverage map that can't be read leaves only the static graph, and an
/// old one misses what changed since; either is worth saying, not failing.
fn coverage_warning(scan: &Scan, config: &Config) -> Option<Warning> {
    let report = scan.coverage.as_ref()?;
    let max = config.graph.coverage.as_ref()?.max_age_days;
    if let Some(why) = &report.error {
        return Some(Warning {
            code: "coverage-unreadable".into(),
            path: Some(report.map.clone()),
            message: format!("the coverage map is left out: {why}"),
        });
    }
    let age = fairlead_core::coverage::days(&fairlead_core::coverage::today())?
        - fairlead_core::coverage::days(&report.created)?;
    (age > i64::from(max)).then(|| Warning {
        code: "coverage-stale".into(),
        path: Some(report.map.clone()),
        message: format!(
            "the coverage map is {age} days old (from {}, commit {}); refresh it, it's allowed {max}",
            report.created,
            report.commit.get(..12).unwrap_or(&report.commit)
        ),
    })
}

/// A provider that failed tells nothing about its files, so a change to one
/// is as uncertain as a run-all path.
fn provider_failed(scan: &Scan, changed: &BTreeSet<String>) -> Option<(String, Warning)> {
    let path = changed.iter().find(|p| scan.uncertain.contains(*p))?;
    let why: Vec<String> = scan
        .providers
        .iter()
        .filter_map(|r| r.failed.as_ref().map(|e| format!("{}: {e}", r.id)))
        .collect();
    let warning = Warning {
        code: "provider-failed".into(),
        path: Some(path.clone()),
        message: format!(
            "a graph provider failed, so every test runs ({})",
            why.join("; ")
        ),
    };
    Some((path.clone(), warning))
}

pub fn start_walk(cx: &Context) -> Walk {
    let graph = &cx.scan.graph;
    let mut starts: Vec<(u32, Via)> = Vec::new();
    let package_files = |manifest: &str, via: &str, starts: &mut Vec<(u32, Via)>| {
        let prefix = format!("{}/", parent(manifest));
        for (id, file) in graph.files.iter().enumerate() {
            if file.starts_with(&prefix) && file != manifest {
                starts.push((id as u32, Via::Path(via.to_string())));
            }
        }
    };
    for path in cx.changed.iter().filter(|p| !cx.ignored.contains(*p)) {
        if let Some(id) = graph.id(path) {
            starts.push((id, Via::Start));
        }
        if is_manifest(cx, path) && !cx.trims.quiet.contains(path) {
            package_files(path, path, &mut starts);
        }
    }
    for (id, via) in &cx.trims.starts {
        starts.push((*id, Via::Path(via.clone())));
    }
    for manifest in cx.lockfile.iter().flat_map(|l| &l.manifests) {
        let via = format!("{LOCKFILE} ({})", parent(manifest));
        if let Some(id) = graph.id(manifest) {
            starts.push((id, Via::Path(via.clone())));
        }
        package_files(manifest, &via, &mut starts);
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
