//! Deleted files: a file a change removed isn't in the head tree, so its
//! importers would lose their edge to it. Each one goes back into the graph
//! as a phantom node, joined to every specifier that fails now but resolves
//! to it when it's shown as an empty file, and to every path literal that
//! names it. The walk then starts from it like any changed file.

use std::collections::HashSet;

use fairlead_core::config::Graph as GraphConfig;
use rayon::prelude::*;

use crate::graph::EdgeKind;
use crate::resolve::Resolver;
use crate::scan::Scan;

/// Adds the deleted paths as phantom nodes and returns their ids, in order.
pub fn attach_deleted(scan: &mut Scan, config: &GraphConfig, deleted: &[String]) -> Vec<u32> {
    let gone: Vec<String> = deleted
        .iter()
        .filter(|p| !scan.tree.contains(p))
        .cloned()
        .collect();
    let ids: Vec<u32> = gone.iter().map(|p| scan.graph.add_phantom(p)).collect();
    if gone.is_empty() {
        return ids;
    }
    let wanted: HashSet<&str> = gone.iter().map(String::as_str).collect();
    let resolver = Resolver::with_phantoms(&scan.tree.root, &scan.packages, config, &gone);
    let tree = &scan.tree;
    let graph = &scan.graph;
    let resolved: Vec<(u32, String, EdgeKind)> = graph
        .failed
        .par_iter()
        .filter_map(|(from, spec, kind)| {
            let file = &graph.files[*from as usize];
            let to = resolver.resolve_path(tree, file, spec)?;
            wanted.contains(to.as_str()).then_some((*from, to, *kind))
        })
        .collect();
    let literals: Vec<(u32, String)> = graph
        .dangling
        .iter()
        .filter(|(_, path)| wanted.contains(path.as_str()))
        .cloned()
        .collect();
    for (from, to, kind) in resolved {
        let to = scan.graph.add_phantom(&to);
        scan.graph.add_edge(from, to, kind);
    }
    for (from, to) in literals {
        let to = scan.graph.add_phantom(&to);
        scan.graph.add_edge(from, to, EdgeKind::PathLiteral);
    }
    ids
}
