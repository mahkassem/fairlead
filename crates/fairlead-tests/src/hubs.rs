//! `graph hubs`: the files that link the most changes to the most tests. A
//! file's feeders are the other files whose change reaches it in the
//! planner's walk, and its tests the unit tests a walk from it reaches; the
//! product counts the change and test pairs that can pass through it, which
//! a barrier on it would cut. The walk's conservative edges, from every file
//! of a module to each file in it that depends on the whole module, go
//! through one extra node per module, so the counts match the walk's.

use std::collections::{HashMap, VecDeque};

use fairlead_core::config::{Config, TestClass};
use fairlead_lang::Scan;

use crate::modules::Modules;
use crate::reach::counts;
use crate::testfiles::discover;
use crate::walk::{is_local, module_dependents};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hub {
    pub path: String,
    pub feeders: u32,
    pub tests: u32,
    pub why: Why,
}

impl Hub {
    pub fn score(&self) -> u64 {
        u64::from(self.feeders) * u64::from(self.tests)
    }
}

/// What makes a hub wide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The file has a conservative edge of its own.
    Own { kind: &'static str, text: String },
    /// A file it depends on has one, and its feeders come through it.
    Through {
        file: String,
        kind: &'static str,
        text: String,
    },
    /// Plain edges only.
    Plain { importers: usize },
}

#[derive(Debug, Default)]
pub struct Hubs {
    pub hubs: Vec<Hub>,
    pub files: usize,
    pub tests: usize,
}

/// The walk as a graph: an edge from a file to each file a change to it
/// reaches in one step, with a node per module standing for its
/// whole-module dependents.
struct WalkGraph {
    succ: Vec<Vec<u32>>,
    pred: Vec<Vec<u32>>,
    barrier: Vec<bool>,
    /// The package a file depends on whole, for files with such an edge.
    package: HashMap<u32, String>,
}

fn walk_graph(scan: &Scan, modules: &Modules) -> WalkGraph {
    let graph = &scan.graph;
    let n = graph.files.len();
    let mut succ: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut virtual_of: HashMap<Option<usize>, u32> = HashMap::new();
    for (module, dependents) in module_dependents(graph, modules) {
        virtual_of.insert(module, succ.len() as u32);
        succ.push(dependents);
    }
    let root = virtual_of.get(&None).copied();
    let mut package = HashMap::new();
    for (id, file) in graph.files.iter().enumerate() {
        for (importer, kind) in graph.importers(id as u32) {
            succ[id].push(importer);
            if kind.is_none() {
                if let Some(name) = graph.package_name_of(id as u32) {
                    package.entry(importer).or_insert_with(|| name.to_string());
                }
            }
        }
        let module = modules.of(file);
        if let Some(&v) = module.and_then(|m| virtual_of.get(&Some(m))) {
            succ[id].push(v);
        }
        succ[id].extend(root);
    }
    let mut pred: Vec<Vec<u32>> = vec![Vec::new(); succ.len()];
    for (from, tos) in succ.iter().enumerate() {
        for &to in tos {
            pred[to as usize].push(from as u32);
        }
    }
    let mut barrier = vec![false; succ.len()];
    for &b in &graph.barrier {
        barrier[b as usize] = true;
    }
    WalkGraph {
        succ,
        pred,
        barrier,
        package,
    }
}

/// The `top` files with the most change and test pairs through them,
/// barrier files left out since the walk already stops at them, and test
/// files.
pub fn hubs(scan: &Scan, config: &Config, top: usize) -> Result<Hubs, String> {
    let graph = &scan.graph;
    let modules = Modules::discover(&scan.tree, &scan.packages, &config.modules)?;
    let found = discover(&scan.tree, config, &modules)?;
    // Only unit tests are selected by the walk; the other classes run when
    // they change, a rule claims them, or always.
    let tests: Vec<u32> = found
        .tests
        .iter()
        .filter(|t| t.class == TestClass::Unit)
        .filter_map(|t| graph.id(&t.path))
        .collect();
    let wg = walk_graph(scan, &modules);
    let n = graph.files.len();
    let files: Vec<u32> = (0..n as u32).collect();
    let reached = counts(&wg.succ, &wg.barrier, &tests);
    let fed = counts(&wg.pred, &wg.barrier, &files);
    // A test file is where a walk ends, so a barrier on one cuts nothing
    // a reader means to cut; it's left out like a barrier file.
    let mut is_test = vec![false; n];
    for id in found.tests.iter().filter_map(|t| graph.id(&t.path)) {
        is_test[id as usize] = true;
    }
    let mut ranked: Vec<(u64, u32, u32, u32)> = (0..n)
        .filter(|&id| !wg.barrier[id] && !is_test[id])
        .map(|id| {
            let (t, f) = (reached[id], fed[id].saturating_sub(1));
            (u64::from(f) * u64::from(t), id as u32, f, t)
        })
        .filter(|(score, ..)| *score > 0)
        .collect();
    // On a tie, as across a cycle, the file with a conservative edge comes
    // first, since a barrier there cuts the cause; then the one most files
    // import directly, the likelier shared module.
    let mut cause = vec![false; n];
    let local = graph.unresolved.iter().filter(|(_, s)| is_local(s));
    let conservative = local.map(|(f, _)| f).chain(&graph.unknown);
    for &id in conservative
        .chain(&graph.tsconfig_fallbacks)
        .chain(wg.package.keys())
    {
        cause[id as usize] = true;
    }
    let first = |id: u32| !cause[id as usize];
    let direct = |id: u32| {
        wg.succ[id as usize]
            .iter()
            .filter(|&&v| (v as usize) < n)
            .count()
    };
    let fan: Vec<usize> = (0..n as u32).map(direct).collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| first(a.1).cmp(&first(b.1)))
            .then_with(|| fan[b.1 as usize].cmp(&fan[a.1 as usize]))
            .then_with(|| graph.files[a.1 as usize].cmp(&graph.files[b.1 as usize]))
    });
    let hubs = ranked
        .into_iter()
        .take(top)
        .map(|(_, id, feeders, tests)| Hub {
            path: graph.files[id as usize].clone(),
            feeders,
            tests,
            why: why(scan, &wg, id),
        })
        .collect();
    Ok(Hubs {
        hubs,
        files: n,
        tests: tests.len(),
    })
}

/// The conservative edge a file has itself, as a kind and the words
/// `test --explain` uses for it.
fn own(scan: &Scan, wg: &WalkGraph, id: u32) -> Option<(&'static str, String)> {
    let graph = &scan.graph;
    let whole = ", so it depends on its whole module";
    if let Some((_, s)) = graph
        .unresolved
        .iter()
        .find(|(f, s)| *f == id && is_local(s))
    {
        return Some((
            "unresolved-import",
            format!("unresolved import `{s}`{whole}"),
        ));
    }
    if graph.unknown.contains(&id) {
        let text = format!("a dynamic import with no literal path{whole}");
        return Some(("dynamic-import", text));
    }
    if graph.tsconfig_fallbacks.contains(&id) {
        let text = format!("its tsconfig couldn't be applied{whole}");
        return Some(("tsconfig-fallback", text));
    }
    let package = wg.package.get(&id)?;
    Some((
        "package",
        format!(
            "an import that resolved to no file, so it depends on every file of package {package}"
        ),
    ))
}

/// Why `id` is wide: its own conservative edge, else the nearest one among
/// the files whose change reaches it, else plain edges.
fn why(scan: &Scan, wg: &WalkGraph, id: u32) -> Why {
    if let Some((kind, text)) = own(scan, wg, id) {
        return Why::Own { kind, text };
    }
    let n = scan.graph.files.len();
    let mut seen = vec![false; wg.pred.len()];
    seen[id as usize] = true;
    let mut queue = VecDeque::from([id]);
    while let Some(at) = queue.pop_front() {
        // A barrier passes on its own change, not the ones that reach it.
        if at != id && wg.barrier[at as usize] {
            continue;
        }
        if at != id && (at as usize) < n {
            if let Some((kind, text)) = own(scan, wg, at) {
                let file = scan.graph.files[at as usize].clone();
                return Why::Through { file, kind, text };
            }
        }
        for &p in &wg.pred[at as usize] {
            if !seen[p as usize] {
                seen[p as usize] = true;
                queue.push_back(p);
            }
        }
    }
    Why::Plain {
        importers: scan.graph.importers(id).len(),
    }
}
