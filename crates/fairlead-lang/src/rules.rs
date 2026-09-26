//! Rule edges and the walk barrier from `[graph]`: dependencies the imports
//! don't show, such as a test that reaches its code over the network, and
//! files the walk reaches but never goes past, such as a server module that
//! imports every area and is imported by every area.

use std::collections::{BTreeSet, HashSet};

use fairlead_core::config::{EdgeRule, Graph as GraphConfig};
use fairlead_core::pattern::{fill, Captures, Pattern};

use crate::graph::{EdgeKind, Graph};

/// More edges than this from one rule almost always means a glob that
/// matches more than was meant, such as `to = ["**"]`.
pub const LARGE_RULE: usize = 50_000;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RuleStats {
    pub edges: usize,
    /// Rules, by `from`, that linked no file.
    pub unmatched: Vec<String>,
    /// Rules, by `from`, past `LARGE_RULE` edges, with their count.
    pub large: Vec<(String, usize)>,
}

pub struct Rules {
    edges: Vec<EdgeRule>,
    barrier: Vec<Pattern>,
}

impl Rules {
    pub fn new(config: &GraphConfig) -> Result<Rules, String> {
        let barrier = config
            .barrier
            .items()
            .iter()
            .map(|g| Pattern::new(g))
            .collect::<Result<_, _>>()?;
        Ok(Rules {
            edges: config.edges.items().to_vec(),
            barrier,
        })
    }

    /// Adds every rule edge to a freshly built graph.
    pub fn apply(&self, graph: &mut Graph) -> Result<RuleStats, String> {
        let mut stats = RuleStats::default();
        for rule in &self.edges {
            let pairs = pairs(graph, rule)?;
            if pairs.is_empty() {
                stats.unmatched.push(rule.from.clone());
            }
            if pairs.len() > LARGE_RULE {
                stats.large.push((rule.from.clone(), pairs.len()));
            }
            stats.edges += pairs.len();
            for (from, to) in pairs {
                graph.add_edge_unchecked(from, to, EdgeKind::Rule);
            }
        }
        Ok(stats)
    }

    /// Adds the rule edges that touch `ids`, files added after the build.
    pub fn apply_to(&self, graph: &mut Graph, ids: &[u32]) -> Result<(), String> {
        let ids: HashSet<u32> = ids.iter().copied().collect();
        for rule in &self.edges {
            for (from, to) in pairs(graph, rule)? {
                if ids.contains(&from) || ids.contains(&to) {
                    graph.add_edge(from, to, EdgeKind::Rule);
                }
            }
        }
        Ok(())
    }

    /// Marks the barrier files among `ids`.
    pub fn mark(&self, graph: &mut Graph, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            if self
                .barrier
                .iter()
                .any(|p| p.is_match(&graph.files[id as usize]))
            {
                graph.barrier.insert(id);
            }
        }
    }
}

fn whole(glob: &str, name: &str) -> bool {
    glob.split('/').any(|s| s == format!("{{{name}}}"))
}

fn names(glob: &str) -> BTreeSet<String> {
    let re = Pattern::new(glob).ok();
    re.and_then(|p| p.captures_names()).unwrap_or_default()
}

/// Every `(from, to)` file pair a rule links. Placeholder values are read
/// from the side where each one is a whole path segment, then both sides
/// are filled with them and matched as globs.
fn pairs(graph: &Graph, rule: &EdgeRule) -> Result<BTreeSet<(u32, u32)>, String> {
    let wanted = names(&rule.from);
    let from_side = wanted.iter().all(|n| whole(&rule.from, n));
    let mut bindings: BTreeSet<Captures> = BTreeSet::new();
    if wanted.is_empty() {
        bindings.insert(Captures::new());
    } else if from_side {
        let from = Pattern::new(&rule.from)?;
        bindings.extend(graph.files.iter().filter_map(|f| from.captures(f)));
    } else {
        for to in rule.to.iter().filter(|t| !names(t).is_empty()) {
            let to = Pattern::new(to)?;
            bindings.extend(graph.files.iter().filter_map(|f| to.captures(f)));
        }
    }
    let matching = |glob: &str| -> Result<Vec<u32>, String> {
        let p = Pattern::new(glob)?;
        Ok((0..graph.files.len() as u32)
            .filter(|&id| p.is_match(&graph.files[id as usize]))
            .collect())
    };
    let mut out = BTreeSet::new();
    for binding in &bindings {
        let from = matching(&fill(&rule.from, binding))?;
        if from.is_empty() {
            continue;
        }
        let mut to = Vec::new();
        for glob in &rule.to {
            to.extend(matching(&fill(glob, binding))?);
        }
        for &f in &from {
            out.extend(to.iter().filter(|&&t| t != f).map(|&t| (f, t)));
        }
    }
    Ok(out)
}
