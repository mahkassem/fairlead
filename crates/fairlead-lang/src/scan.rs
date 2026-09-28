//! Building the graph: list the tree, find the workspace packages, extract
//! and resolve every source file in parallel, then add the edges.

use std::collections::HashMap;
use std::path::Path;

use fairlead_core::config::Config;
use rayon::prelude::*;

use crate::cache::{self, CacheStats, ParseCache};
use crate::extract::{extract, Extracted, SpecKind};
use crate::graph::{EdgeKind, Graph};
use crate::php;
use crate::provider;
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
    /// A PHP file's fully qualified references, resolved once every file
    /// is read, and the names it declares.
    names: Vec<String>,
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
    let scanned = sources.len();
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
    let autoload = php::Autoload::new(
        &tree,
        results
            .iter()
            .map(|(f, r)| (f.as_str(), r.declares.as_slice())),
    );
    let (builtin_edges, php_edges) = add_results(&tree, &autoload, &mut graph, results);
    let php_files = sources.iter().filter(|f| is_php(f)).count();
    let mut providers = vec![provider::Report {
        id: provider::BUILTIN.into(),
        files: scanned - php_files,
        edges: builtin_edges,
        ignored: 0,
        failed: None,
    }];
    if php_files > 0 {
        providers.push(provider::Report {
            id: php::ID.into(),
            files: php_files,
            edges: php_edges,
            ignored: 0,
            failed: None,
        });
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
    })
}

fn is_php(file: &str) -> bool {
    file.ends_with(".php")
}

/// Adds each file's edges, resolving PHP names now that every declaration
/// is known; returns the edges added for the built-in scanner and for PHP.
fn add_results(
    tree: &Tree,
    autoload: &php::Autoload,
    graph: &mut Graph,
    results: Vec<(String, FileResult)>,
) -> (usize, usize) {
    let (mut builtin, mut php_edges) = (0, 0);
    for (file, result) in results {
        let from = graph.id(&file).expect("every source is in the tree");
        let before = graph.dependencies(from).len();
        for (to, kind) in result.edges {
            if let Some(to) = graph.id(&to) {
                graph.add_edge(from, to, kind);
            }
        }
        for name in &result.names {
            match autoload.resolve(tree, name) {
                php::Resolved::Files(files) => {
                    let ids: Vec<u32> = files.iter().filter_map(|f| graph.id(f)).collect();
                    for to in ids {
                        if to != from {
                            graph.add_edge(from, to, EdgeKind::Import);
                        }
                    }
                }
                php::Resolved::Missing(paths) => {
                    graph.dangling.extend(paths.into_iter().map(|p| (from, p)));
                }
                php::Resolved::External => {}
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
        let added = graph.dependencies(from).len() - before;
        if is_php(&file) {
            php_edges += added;
        } else {
            builtin += added;
        }
    }
    (builtin, php_edges)
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
    if is_php(file) {
        result.names = extracted.specs.iter().map(|(n, _)| n.clone()).collect();
        result.declares = extracted.declares.clone();
    }
    for (spec, kind) in extracted.specs.iter().filter(|_| !is_php(file)) {
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
