//! Building the graph: list the tree, find the workspace packages, extract
//! and resolve every source file in parallel, then add the edges.

use std::collections::HashMap;
use std::path::Path;

use fairlead_core::config::Config;
use rayon::prelude::*;

use crate::cache::{self, CacheStats, ParseCache};
use crate::extract::{extract, Extracted, SpecKind};
use crate::golang;
use crate::graph::{EdgeKind, Graph};
use crate::php;
use crate::provider;
use crate::python;
use crate::resolve::{Resolver, Target};
use crate::rules::{RuleStats, Rules};
use crate::tree::{normalize, parent, Tree};
use crate::workspace::{self, Package};

#[derive(Default)]
struct FileResult {
    edges: Vec<(String, EdgeKind)>,
    packages: Vec<String>,
    unresolved: Vec<String>,
    /// Specifiers that didn't resolve, with their edge kind.
    failed: Vec<(String, EdgeKind)>,
    /// Path-like literals naming files that aren't in the tree.
    dangling: Vec<String>,
    unknown: bool,
    fell_back: bool,
    parsed: Option<Parsed>,
    /// A PHP or Go file's references by name, resolved once every file is
    /// read, and the names it declares.
    names: Vec<(String, SpecKind)>,
    declares: Vec<String>,
}

/// A file's extraction, with its cache key when the cache is on.
struct Parsed {
    extracted: Extracted,
    key: Option<String>,
    hit: bool,
}

pub struct Scan {
    pub tree: Tree,
    pub graph: Graph,
    pub packages: Vec<Package>,
    pub cache: CacheStats,
    pub rules: RuleStats,
    /// What each provider contributed, the built-in scanner first.
    pub providers: Vec<provider::Report>,
    /// Files two external providers both claimed.
    pub conflicts: Vec<provider::Conflict>,
    /// Files claimed by a provider that failed: nothing is known about them.
    pub uncertain: std::collections::HashSet<String>,
    /// What the coverage map added, when one is configured.
    pub coverage: Option<crate::coverage::Report>,
    /// How PHP names resolve here, for turning a coverage run's test names into files.
    pub autoload: php::Autoload,
}

pub fn build(root: &Path, config: &Config) -> std::io::Result<Scan> {
    // Canonical and plain, so resolved paths (which the resolver canonicalizes) share its prefix.
    let root = crate::tree::plain(&std::fs::canonicalize(root)?);
    let tree = Tree::scan(&root);
    let packages = workspace::discover(&tree);
    let resolver = Resolver::new(&root, &packages, &config.graph);
    let named = packages
        .iter()
        .map(|p| (p.name.clone(), p.dir.clone()))
        .collect();
    let mut graph = Graph::with_files(tree.files.clone(), named);
    let parse_cache = if config.graph.cache {
        ParseCache::open(&root)
    } else {
        ParseCache::disabled()
    };
    let externals = config.graph.providers.items();
    let (owner, conflicts) = if externals.is_empty() {
        Default::default()
    } else {
        provider::claims(&tree.files, externals).map_err(std::io::Error::other)?
    };
    let sources: Vec<&String> = tree.sources().filter(|f| !owner.contains_key(*f)).collect();
    let mut results: Vec<(String, FileResult)> = sources
        .par_iter()
        .map(|file| {
            let result = scan_file(
                &tree,
                &resolver,
                &parse_cache,
                file,
                config.graph.type_imports,
            );
            ((*file).clone(), result)
        })
        .collect();
    let mut stats = CacheStats {
        enabled: parse_cache.enabled(),
        ..CacheStats::default()
    };
    let mut used = HashMap::new();
    for (_, result) in &mut results {
        if let Some(Parsed {
            extracted,
            key: Some(key),
            hit,
        }) = result.parsed.take()
        {
            if hit {
                stats.hits += 1;
            } else {
                stats.misses += 1;
            }
            used.insert(key, extracted);
        }
    }
    parse_cache.save(used);
    let named = Named {
        autoload: php::Autoload::new(
            &tree,
            results
                .iter()
                .map(|(f, r)| (f.as_str(), r.declares.as_slice())),
        ),
        modules: golang::Modules::new(&tree),
        roots: python::Roots::new(&tree),
    };
    let edges = add_results(&tree, &named, &mut graph, results);
    let coverage = config
        .graph
        .coverage
        .as_ref()
        .map(|c| crate::coverage::apply(&root, c, &mut graph));
    let mut providers = Vec::new();
    for id in [provider::BUILTIN, php::ID, golang::ID, python::ID] {
        let files = sources.iter().filter(|f| language(f) == id).count();
        if files > 0 || id == provider::BUILTIN {
            providers.push(provider::Report {
                id: id.into(),
                files,
                edges: edges.get(id).copied().unwrap_or(0),
                ignored: 0,
                failed: None,
            });
        }
    }
    let uncertain = run_externals(&root, externals, &owner, &mut graph, &mut providers);
    add_snapshot_edges(&tree, &mut graph);
    let rules = Rules::new(&config.graph).map_err(std::io::Error::other)?;
    let rule_stats = rules.apply(&mut graph).map_err(std::io::Error::other)?;
    let all = 0..graph.files.len() as u32;
    rules.mark(&mut graph, all);
    Ok(Scan {
        tree,
        graph,
        packages,
        cache: stats,
        rules: rule_stats,
        providers,
        conflicts,
        uncertain,
        coverage,
        autoload: named.autoload,
    })
}

/// Which built-in scanner reads a file, by its id in reports.
fn language(file: &str) -> &'static str {
    if file.ends_with(".php") {
        php::ID
    } else if file.ends_with(".go") {
        golang::ID
    } else if file.ends_with(".py") {
        python::ID
    } else {
        provider::BUILTIN
    }
}

/// What turns a name into files, for languages that refer by name: known
/// only once every file is read.
struct Named {
    autoload: php::Autoload,
    modules: golang::Modules,
    roots: python::Roots,
}

impl Named {
    /// The files a reference reaches, and the paths it would need that
    /// aren't there.
    fn resolve(
        &self,
        tree: &Tree,
        file: &str,
        name: &str,
        kind: SpecKind,
    ) -> (Vec<String>, Vec<String>) {
        match language(file) {
            php::ID => match self.autoload.resolve(tree, name) {
                php::Resolved::Files(files) => (files, Vec::new()),
                php::Resolved::Missing(paths) => (Vec::new(), paths),
                php::Resolved::External => Default::default(),
            },
            golang::ID if kind == SpecKind::Require => {
                (self.modules.embedded(tree, file, name), Vec::new())
            }
            golang::ID => (self.modules.package(file, name), Vec::new()),
            python::ID => (self.roots.resolve(tree, file, name), Vec::new()),
            _ => Default::default(),
        }
    }
}

/// Adds each file's edges, resolving names now that every file is read;
/// returns the edges added by each built-in scanner.
fn add_results(
    tree: &Tree,
    named: &Named,
    graph: &mut Graph,
    results: Vec<(String, FileResult)>,
) -> HashMap<&'static str, usize> {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for (file, result) in results {
        let from = graph.id(&file).expect("every source is in the tree");
        let before = graph.dependencies(from).len();
        for (to, kind) in result.edges {
            if let Some(to) = graph.id(&to) {
                graph.add_edge(from, to, kind);
            }
        }
        let mut reached = Vec::new();
        for (name, kind) in &result.names {
            let (files, missing) = named.resolve(tree, &file, name, *kind);
            let kind = if *kind == SpecKind::Require && language(&file) == golang::ID {
                EdgeKind::PathLiteral
            } else {
                EdgeKind::Import
            };
            reached.extend(files.into_iter().map(|f| (f, kind)));
            graph
                .dangling
                .extend(missing.into_iter().map(|p| (from, p)));
        }
        if python::is_test(&file) {
            reached.extend(
                python::conftests(tree, &file)
                    .into_iter()
                    .map(|f| (f, EdgeKind::Import)),
            );
        }
        if language(&file) == golang::ID {
            reached.extend(
                named
                    .modules
                    .manifests(tree, &file)
                    .into_iter()
                    .map(|f| (f, EdgeKind::Manifest)),
            );
        }
        if golang::is_test(&file) {
            reached.extend(
                named
                    .modules
                    .siblings(&file)
                    .into_iter()
                    .map(|f| (f, EdgeKind::Import)),
            );
        }
        for (to, kind) in reached {
            if let Some(to) = graph.id(&to).filter(|&to| to != from) {
                graph.add_edge(from, to, kind);
            }
        }
        for package in result.packages {
            graph.add_package_edge(from, &package);
        }
        graph
            .unresolved
            .extend(result.unresolved.into_iter().map(|s| (from, s)));
        graph
            .failed
            .extend(result.failed.into_iter().map(|(s, k)| (from, s, k)));
        graph
            .dangling
            .extend(result.dangling.into_iter().map(|p| (from, p)));
        if result.unknown {
            graph.unknown.push(from);
        }
        if result.fell_back {
            graph.tsconfig_fallbacks.push(from);
        }
        *counts.entry(language(&file)).or_default() += graph.dependencies(from).len() - before;
    }
    counts
}

/// Runs each external provider on the files it claims; returns the files of
/// those that failed, about which nothing is known.
fn run_externals(
    root: &Path,
    externals: &[fairlead_core::config::GraphProvider],
    owner: &HashMap<String, usize>,
    graph: &mut Graph,
    reports: &mut Vec<provider::Report>,
) -> std::collections::HashSet<String> {
    let mut uncertain = std::collections::HashSet::new();
    for (i, p) in externals.iter().enumerate() {
        let mut claimed: Vec<String> = owner
            .iter()
            .filter(|(_, &o)| o == i)
            .map(|(f, _)| f.clone())
            .collect();
        claimed.sort();
        if claimed.is_empty() {
            continue;
        }
        let report = provider::run(root, p, &claimed, graph);
        if report.failed.is_some() {
            uncertain.extend(claimed);
        }
        reports.push(report);
    }
    uncertain
}

/// The file's extraction, from the cache when its bytes haven't changed.
fn parse(cache: &ParseCache, file: &str, source: &[u8]) -> Parsed {
    if !cache.enabled() {
        return Parsed {
            extracted: extract(file, source),
            key: None,
            hit: false,
        };
    }
    let key = cache::key(file, source);
    let cached = cache.get(&key).cloned();
    Parsed {
        hit: cached.is_some(),
        extracted: cached.unwrap_or_else(|| extract(file, source)),
        key: Some(key),
    }
}

fn scan_file(
    tree: &Tree,
    resolver: &Resolver,
    cache: &ParseCache,
    file: &str,
    type_imports: bool,
) -> FileResult {
    let Ok(source) = std::fs::read(tree.abs(file)) else {
        return FileResult::default();
    };
    let parsed = parse(cache, file, &source);
    let extracted = &parsed.extracted;
    let mut result = FileResult {
        unknown: extracted.unknown_dynamic,
        ..FileResult::default()
    };
    let by_name = language(file) != provider::BUILTIN;
    if by_name {
        result.names = extracted.specs.clone();
        result.declares = extracted.declares.clone();
    }
    for (spec, kind) in extracted.specs.iter().filter(|_| !by_name) {
        if *kind == SpecKind::TypeImport && !type_imports {
            continue;
        }
        let (target, fell_back) = resolver.resolve(tree, file, spec);
        result.fell_back |= fell_back;
        match target {
            Target::File(to) => result.edges.push((to, (*kind).into())),
            Target::Package(name) => result.packages.push(name),
            Target::External => {}
            Target::NotFound => result.failed.push((spec.clone(), (*kind).into())),
            Target::Unresolved => {
                result.unresolved.push(spec.clone());
                result.failed.push((spec.clone(), (*kind).into()));
            }
        }
    }
    for literal in &extracted.literals {
        let from_file = normalize(parent(file), literal);
        let from_root = normalize("", literal.trim_start_matches("./"));
        let candidates: Vec<String> = [from_file, from_root].into_iter().flatten().collect();
        match candidates.iter().find(|p| tree.contains(p)) {
            Some(to) => result.edges.push((to.clone(), EdgeKind::PathLiteral)),
            None => result.dangling.extend(candidates),
        }
    }
    result.parsed = Some(parsed);
    result
}

/// `dir/__snapshots__/x.test.ts.snap` belongs to `dir/x.test.ts`.
fn add_snapshot_edges(tree: &Tree, graph: &mut Graph) {
    for snap in tree.files.iter().filter(|f| f.ends_with(".snap")) {
        let dir = parent(snap);
        if !(dir == "__snapshots__" || dir.ends_with("/__snapshots__")) {
            continue;
        }
        let name = snap
            .rsplit('/')
            .next()
            .unwrap_or(snap)
            .trim_end_matches(".snap");
        let Some(test) = normalize(parent(dir), name) else {
            continue;
        };
        if let (Some(from), Some(to)) = (graph.id(&test), graph.id(snap)) {
            graph.add_edge(from, to, EdgeKind::Snapshot);
        }
    }
}
