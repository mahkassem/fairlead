//! Building the graph: list the tree, find the workspace packages, extract
//! and resolve every source file in parallel, then add the edges.

use std::path::Path;

use fairlead_core::config::Config;
use rayon::prelude::*;

use crate::extract::{extract, SpecKind};
use crate::graph::{EdgeKind, Graph};
use crate::resolve::{Resolver, Target};
use crate::tree::{normalize, parent, Tree};
use crate::workspace;

#[derive(Default)]
struct FileResult {
    edges: Vec<(String, EdgeKind)>,
    packages: Vec<String>,
    unresolved: Vec<String>,
    unknown: bool,
    fell_back: bool,
}

pub struct Scan {
    pub tree: Tree,
    pub graph: Graph,
}

pub fn build(root: &Path, config: &Config) -> std::io::Result<Scan> {
    // Canonical, so resolved paths (which the resolver canonicalizes) share its prefix.
    let root = std::fs::canonicalize(root)?;
    let tree = Tree::scan(&root);
    let packages = workspace::discover(&tree);
    let resolver = Resolver::new(&root, &packages, &config.graph);
    let named = packages
        .iter()
        .map(|p| (p.name.clone(), p.dir.clone()))
        .collect();
    let mut graph = Graph::with_files(tree.files.clone(), named);
    let sources: Vec<&String> = tree.sources().collect();
    let results: Vec<(String, FileResult)> = sources
        .par_iter()
        .map(|file| {
            (
                (*file).clone(),
                scan_file(&tree, &resolver, file, config.graph.type_imports),
            )
        })
        .collect();
    for (file, result) in results {
        let from = graph.id(&file).expect("every source is in the tree");
        for (to, kind) in result.edges {
            if let Some(to) = graph.id(&to) {
                graph.add_edge(from, to, kind);
            }
        }
        for package in result.packages {
            graph.add_package_edge(from, &package);
        }
        graph
            .unresolved
            .extend(result.unresolved.into_iter().map(|s| (from, s)));
        if result.unknown {
            graph.unknown.push(from);
        }
        if result.fell_back {
            graph.tsconfig_fallbacks.push(from);
        }
    }
    add_snapshot_edges(&tree, &mut graph);
    Ok(Scan { tree, graph })
}

fn scan_file(tree: &Tree, resolver: &Resolver, file: &str, type_imports: bool) -> FileResult {
    let Ok(source) = std::fs::read(tree.abs(file)) else {
        return FileResult::default();
    };
    let extracted = extract(file, &source);
    let mut result = FileResult {
        unknown: extracted.unknown_dynamic,
        ..FileResult::default()
    };
    for (spec, kind) in &extracted.specs {
        if *kind == SpecKind::TypeImport && !type_imports {
            continue;
        }
        let (target, fell_back) = resolver.resolve(tree, file, spec);
        result.fell_back |= fell_back;
        match target {
            Target::File(to) => result.edges.push((to, (*kind).into())),
            Target::Package(name) => result.packages.push(name),
            Target::External => {}
            Target::Unresolved => result.unresolved.push(spec.clone()),
        }
    }
    for literal in &extracted.literals {
        let from_file = normalize(parent(file), literal);
        let from_root = normalize("", literal.trim_start_matches("./"));
        if let Some(to) = [from_file, from_root]
            .into_iter()
            .flatten()
            .find(|p| tree.contains(p))
        {
            result.edges.push((to, EdgeKind::PathLiteral));
        }
    }
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
