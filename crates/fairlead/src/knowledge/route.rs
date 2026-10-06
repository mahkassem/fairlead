//! One router for every kind of knowledge: an entry's scope covers a
//! changed path (named), a file a changed file imports (used), or every
//! change (always); with no graph to follow, everything is offered as the
//! fallback, as a run of everything does for tests.
//!
//! Reach goes along imports, not importers: a file needs the contracts it
//! uses, and nothing imports a test, so a test's rules are only reachable
//! forward. The walk stops at a graph barrier, so a barrel that re-exports
//! a package doesn't pull in everything behind it.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use fairlead_core::pattern::Pattern;
use fairlead_lang::graph::Graph;

#[derive(Debug, Clone)]
pub struct Scope {
    pub paths: Vec<Pattern>,
    pub modules: Vec<String>,
    pub always: bool,
}

impl Scope {
    pub fn new(paths: &[String], modules: &[String], always: bool) -> Result<Scope, String> {
        let paths = paths
            .iter()
            .map(|p| Pattern::new(p).map_err(|e| format!("`paths` entry `{p}`: {e}")))
            .collect::<Result<_, _>>()?;
        Ok(Scope {
            paths,
            modules: modules.to_vec(),
            always,
        })
    }

    pub fn covers(&self, path: &str, module: Option<&str>) -> bool {
        self.paths.iter().any(|p| p.is_match(path))
            || module.is_some_and(|m| self.modules.iter().any(|s| s == m))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// The scope covers a changed path.
    Named {
        path: String,
    },
    /// The scope covers `path`, which `via` imports.
    Used {
        path: String,
        via: String,
    },
    /// The scope covers `path`, which imports `of`; only with `skills.importers`.
    Importer {
        path: String,
        of: String,
    },
    Always,
    /// Nothing to follow, so everything is offered.
    Fallback,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Named { path } => write!(f, "named: {path}"),
            Reason::Used { path, via } => write!(f, "used: {via} imports {path}"),
            Reason::Importer { path, of } => write!(f, "importer: {path} imports {of}"),
            Reason::Always => f.write_str("always"),
            Reason::Fallback => f.write_str("fallback: nothing to follow, so everything"),
        }
    }
}

/// How far a change reaches past the paths it names.
#[derive(Debug, Clone, Copy)]
pub struct Hops {
    pub imports: usize,
    pub importers: usize,
}

/// What a change reaches, worked out once and asked of every entry.
pub struct Reach {
    changed: Vec<(String, Option<String>)>,
    /// A file a changed file imports, within the hops, and the changed file it came from.
    used: BTreeMap<String, (Option<String>, String)>,
    /// A file that imports a changed file, within the hops, and that changed file.
    importers: BTreeMap<String, (Option<String>, String)>,
    fallback: bool,
}

/// Files within `hops` of `start` along `next`, each with the changed file
/// it came from; the walk doesn't go past a barrier.
fn walk(
    start: &str,
    changed: &[String],
    graph: &Graph,
    hops: usize,
    next: &dyn Fn(u32) -> Vec<u32>,
    module_of: &dyn Fn(&str) -> Option<String>,
    out: &mut BTreeMap<String, (Option<String>, String)>,
) {
    let Some(id) = graph.id(start) else { return };
    let mut queue = VecDeque::from([(id, 0usize)]);
    let mut seen = vec![id];
    while let Some((file, depth)) = queue.pop_front() {
        if depth == hops || (depth > 0 && graph.is_barrier(file)) {
            continue;
        }
        for other in next(file) {
            if seen.contains(&other) {
                continue;
            }
            seen.push(other);
            let path = graph.files[other as usize].clone();
            if !changed.contains(&path) {
                out.entry(path.clone())
                    .or_insert_with(|| (module_of(&path), start.to_string()));
            }
            queue.push_back((other, depth + 1));
        }
    }
}

impl Reach {
    /// `module_of` names a path's module, for scopes given by module.
    /// `fallback` is set when the change can't be followed, such as a run
    /// of everything.
    pub fn new(
        changed: &[String],
        graph: &Graph,
        hops: Hops,
        module_of: &dyn Fn(&str) -> Option<String>,
        fallback: bool,
    ) -> Reach {
        let mut used = BTreeMap::new();
        let mut importers = BTreeMap::new();
        let forward = |f: u32| graph.dependencies(f).iter().map(|&(d, _)| d).collect();
        let back = |f: u32| graph.importers(f).into_iter().map(|(i, _)| i).collect();
        for start in changed {
            walk(
                start,
                changed,
                graph,
                hops.imports,
                &forward,
                module_of,
                &mut used,
            );
            walk(
                start,
                changed,
                graph,
                hops.importers,
                &back,
                module_of,
                &mut importers,
            );
        }
        Reach {
            changed: changed.iter().map(|p| (p.clone(), module_of(p))).collect(),
            used,
            importers,
            fallback,
        }
    }

    /// Why `scope` applies to this change, the strongest reason first, or
    /// `None` when it doesn't.
    pub fn reason(&self, scope: &Scope) -> Option<Reason> {
        if let Some((path, _)) = self
            .changed
            .iter()
            .find(|(p, m)| scope.covers(p, m.as_deref()))
        {
            return Some(Reason::Named { path: path.clone() });
        }
        if let Some((path, (_, via))) = self
            .used
            .iter()
            .find(|(p, (m, _))| scope.covers(p, m.as_deref()))
        {
            return Some(Reason::Used {
                path: path.clone(),
                via: via.clone(),
            });
        }
        if let Some((path, (_, of))) = self
            .importers
            .iter()
            .find(|(p, (m, _))| scope.covers(p, m.as_deref()))
        {
            return Some(Reason::Importer {
                path: path.clone(),
                of: of.clone(),
            });
        }
        if scope.always {
            return Some(Reason::Always);
        }
        self.fallback.then_some(Reason::Fallback)
    }
}

/// Entries with the reason each applies, in the order a brief lists them:
/// by reason, then by `rank` (lower first) within one.
pub fn select<T, K: Ord>(
    entries: &[T],
    scope: impl Fn(&T) -> &Scope,
    rank: impl Fn(&T) -> K,
    reach: &Reach,
) -> Vec<(usize, Reason)> {
    let mut out: Vec<(usize, Reason)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| reach.reason(scope(e)).map(|r| (i, r)))
        .collect();
    let group = |r: &Reason| match r {
        Reason::Named { .. } => 0,
        Reason::Used { .. } | Reason::Importer { .. } => 1,
        Reason::Always => 2,
        Reason::Fallback => 3,
    };
    out.sort_by(|(a, ra), (b, rb)| {
        group(ra)
            .cmp(&group(rb))
            .then_with(|| rank(&entries[*a]).cmp(&rank(&entries[*b])))
            .then(a.cmp(b))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fairlead_lang::graph::EdgeKind;

    /// test/a.test.ts imports src/a.ts, which imports src/index.ts (a
    /// barrier), which imports src/deep.ts.
    fn graph() -> Graph {
        let files = ["src/a.ts", "src/index.ts", "src/deep.ts", "test/a.test.ts"];
        let mut g = Graph::with_files(files.iter().map(|s| s.to_string()).collect(), Vec::new());
        g.add_edge(3, 0, EdgeKind::Import);
        g.add_edge(0, 1, EdgeKind::Import);
        g.add_edge(1, 2, EdgeKind::Import);
        g
    }

    fn scope(paths: &[&str], always: bool) -> Scope {
        let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        Scope::new(&paths, &[], always).unwrap()
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    fn hops(imports: usize, importers: usize) -> Hops {
        Hops { imports, importers }
    }

    #[test]
    fn named_beats_used_beats_always_and_fallback_only_when_set() {
        let g = graph();
        let reach = Reach::new(&["test/a.test.ts".into()], &g, hops(1, 0), &none, false);
        assert_eq!(
            reach.reason(&scope(&["test/**"], true)),
            Some(Reason::Named {
                path: "test/a.test.ts".into()
            })
        );
        assert_eq!(
            reach.reason(&scope(&["src/a.ts"], true)),
            Some(Reason::Used {
                path: "src/a.ts".into(),
                via: "test/a.test.ts".into()
            })
        );
        assert_eq!(
            reach.reason(&scope(&["docs/**"], true)),
            Some(Reason::Always)
        );
        assert_eq!(reach.reason(&scope(&["docs/**"], false)), None);
        let fallback = Reach::new(&["package.json".into()], &g, hops(1, 0), &none, true);
        assert_eq!(
            fallback.reason(&scope(&["docs/**"], false)),
            Some(Reason::Fallback)
        );
    }

    #[test]
    fn reach_goes_along_imports_never_importers() {
        let g = graph();
        let reach = Reach::new(&["src/a.ts".into()], &g, hops(1, 0), &none, false);
        assert_eq!(
            reach.reason(&scope(&["test/**"], false)),
            None,
            "a test imports a.ts"
        );
        assert!(reach.reason(&scope(&["src/index.ts"], false)).is_some());
    }

    #[test]
    fn hops_count_and_a_barrier_stops_the_walk() {
        let mut g = graph();
        let deep = scope(&["src/deep.ts"], false);
        let two = Reach::new(&["src/a.ts".into()], &g, hops(2, 0), &none, false);
        assert!(two.reason(&deep).is_some(), "two hops reach deep.ts");
        let one = Reach::new(&["src/a.ts".into()], &g, hops(1, 0), &none, false);
        assert!(one.reason(&deep).is_none());
        g.barrier.insert(1);
        let barred = Reach::new(&["src/a.ts".into()], &g, hops(2, 0), &none, false);
        assert!(
            barred.reason(&deep).is_none(),
            "the walk stops at the barrel"
        );
    }

    #[test]
    fn importers_are_followed_only_when_asked_and_rank_after_imports() {
        let g = graph();
        let tests = scope(&["test/**"], false);
        let off = Reach::new(&["src/a.ts".into()], &g, hops(1, 0), &none, false);
        assert_eq!(off.reason(&tests), None);
        let on = Reach::new(&["src/a.ts".into()], &g, hops(1, 1), &none, false);
        assert_eq!(
            on.reason(&tests),
            Some(Reason::Importer {
                path: "test/a.test.ts".into(),
                of: "src/a.ts".into()
            })
        );
        let both = scope(&["test/**", "src/index.ts"], false);
        assert!(matches!(on.reason(&both), Some(Reason::Used { .. })));
    }

    #[test]
    fn a_module_scope_covers_the_modules_files() {
        let g = graph();
        let module_of = |p: &str| p.starts_with("src/").then(|| "core".to_string());
        let reach = Reach::new(&["src/a.ts".into()], &g, hops(1, 0), &module_of, false);
        let by_module = Scope::new(&[], &["core".into()], false).unwrap();
        assert!(matches!(
            reach.reason(&by_module),
            Some(Reason::Named { .. })
        ));
    }

    #[test]
    fn selection_orders_by_reason_then_rank() {
        let g = graph();
        let reach = Reach::new(&["test/a.test.ts".into()], &g, hops(1, 0), &none, false);
        let entries = [
            (scope(&["docs/**"], true), 0),
            (scope(&["src/a.ts"], false), 1),
            (scope(&["test/**"], false), 9),
            (scope(&["test/**"], false), 2),
        ];
        let picked: Vec<usize> = select(&entries, |e| &e.0, |e| e.1, &reach)
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert_eq!(picked, [3, 2, 1, 0]);
    }
}
