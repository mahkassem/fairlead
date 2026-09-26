//! The reverse walk: from the changed files to everything that depends on
//! them, over import, path-literal, snapshot and package edges. Conservative
//! edges come on top: a file with a non-literal dynamic import, a local
//! specifier that didn't resolve, or a tsconfig that couldn't be applied
//! depends on every file in its module, since no edge says which one it
//! needs. At the root, outside any module, that means every file.

use std::collections::{HashMap, HashSet, VecDeque};

use fairlead_lang::resolve::package_name;
use fairlead_lang::Graph;

use crate::modules::Modules;

/// How a file was reached: from another file, or from a path that isn't in
/// the graph (a deleted file, or a changed manifest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Via {
    Start,
    File(u32),
    Path(String),
}

#[derive(Debug, Default)]
pub struct Walk {
    pub reached: HashMap<u32, Via>,
}

impl Walk {
    /// The chain from the change that reached `id` to `id` itself.
    pub fn chain(&self, graph: &Graph, id: u32) -> Vec<String> {
        let mut chain = vec![graph.files[id as usize].clone()];
        let mut at = id;
        loop {
            match self.reached.get(&at) {
                Some(Via::File(from)) => {
                    chain.push(graph.files[*from as usize].clone());
                    at = *from;
                }
                Some(Via::Path(path)) => {
                    chain.push(path.clone());
                    break;
                }
                _ => break,
            }
        }
        chain.reverse();
        chain
    }
}

/// Is `spec` local, so that failing to resolve it means a missing file here
/// rather than a missing install?
pub fn is_local(spec: &str) -> bool {
    spec.starts_with('.')
        || spec.starts_with('/')
        || spec.starts_with('#')
        || package_name(spec).is_none()
}

/// Files that depend on their whole module, keyed by module (`None` is the root).
pub fn module_dependents(graph: &Graph, modules: &Modules) -> HashMap<Option<usize>, Vec<u32>> {
    let mut out: HashMap<Option<usize>, Vec<u32>> = HashMap::new();
    let unresolved_local = graph
        .unresolved
        .iter()
        .filter(|(_, spec)| is_local(spec))
        .map(|(from, _)| *from);
    let mut seen = HashSet::new();
    let fallbacks = graph.tsconfig_fallbacks.iter().copied();
    for id in graph
        .unknown
        .iter()
        .copied()
        .chain(unresolved_local)
        .chain(fallbacks)
    {
        if seen.insert(id) {
            let module = modules.of(&graph.files[id as usize]);
            out.entry(module).or_default().push(id);
        }
    }
    out
}

pub fn walk(graph: &Graph, modules: &Modules, starts: Vec<(u32, Via)>) -> Walk {
    walk_until(graph, modules, starts, |_| false).0
}

/// The walk, stopping as soon as `stop` holds for a reached file; says
/// whether it stopped.
pub fn walk_until(
    graph: &Graph,
    modules: &Modules,
    starts: Vec<(u32, Via)>,
    stop: impl Fn(u32) -> bool,
) -> (Walk, bool) {
    let dependents = module_dependents(graph, modules);
    let root_dependents = dependents.get(&None).cloned().unwrap_or_default();
    let mut walk = Walk::default();
    let mut queue = VecDeque::new();
    for (id, via) in starts {
        if let std::collections::hash_map::Entry::Vacant(slot) = walk.reached.entry(id) {
            slot.insert(via);
            if stop(id) {
                return (walk, true);
            }
            queue.push_back(id);
        }
    }
    let mut widened: HashSet<Option<usize>> = HashSet::new();
    let mut root_widened = false;
    while let Some(file) = queue.pop_front() {
        let mut next: Vec<u32> = graph.importers(file).into_iter().map(|(f, _)| f).collect();
        let module = modules.of(&graph.files[file as usize]);
        if widened.insert(module) {
            next.extend(dependents.get(&module).into_iter().flatten().copied());
        }
        if !root_widened {
            root_widened = true;
            next.extend(root_dependents.iter().copied());
        }
        for importer in next {
            if let std::collections::hash_map::Entry::Vacant(slot) = walk.reached.entry(importer) {
                slot.insert(Via::File(file));
                if stop(importer) {
                    return (walk, true);
                }
                queue.push_back(importer);
            }
        }
    }
    (walk, false)
}
