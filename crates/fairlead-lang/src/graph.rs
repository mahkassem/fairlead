//! The file graph. An edge from A to B means A depends on B, so walking
//! edges backwards from a changed file finds everything it can affect. A
//! package edge stands for an edge to every file in that package.
//!
//! A file the walk reaches only over re-export edges changed in something
//! it re-exports, not in its own code, so it doesn't go on to an importer
//! whose named import was resolved past it to the files that declare the
//! names: that importer has edges to those files already.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::extract::SpecKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKind {
    Import,
    TypeImport,
    /// `export … from`: the module passes the target's names on.
    ReExport,
    TypeReExport,
    Dynamic,
    Require,
    Mock,
    /// A string literal naming a file in the repository.
    PathLiteral,
    /// `__snapshots__/<test>.snap` to its test.
    Snapshot,
    /// An edge an external `[[graph.providers]]` command printed.
    Provider,
    /// A test ran this file in the coverage map's run.
    Coverage,
    /// A Go file to its module's `go.mod` and `go.sum`, which pick the
    /// versions of everything it imports.
    Manifest,
    /// A `[[graph.edges]]` rule. Last, so an import between the same two
    /// files names itself first.
    Rule,
}

impl EdgeKind {
    /// How a reader names the edge: in `graph why` and `test --explain`.
    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::Import => "import",
            EdgeKind::TypeImport => "type import",
            EdgeKind::ReExport => "re-export",
            EdgeKind::TypeReExport => "type re-export",
            EdgeKind::Dynamic => "dynamic import",
            EdgeKind::Require => "require",
            EdgeKind::Mock => "mock",
            EdgeKind::PathLiteral => "path literal",
            EdgeKind::Snapshot => "snapshot",
            EdgeKind::Provider => "provider",
            EdgeKind::Coverage => "coverage",
            EdgeKind::Manifest => "manifest",
            EdgeKind::Rule => "[[graph.edges]] rule",
        }
    }

    pub fn is_reexport(self) -> bool {
        matches!(self, EdgeKind::ReExport | EdgeKind::TypeReExport)
    }
}

impl From<SpecKind> for EdgeKind {
    fn from(kind: SpecKind) -> Self {
        match kind {
            SpecKind::Import => EdgeKind::Import,
            SpecKind::TypeImport => EdgeKind::TypeImport,
            SpecKind::Dynamic => EdgeKind::Dynamic,
            SpecKind::Require => EdgeKind::Require,
            SpecKind::Mock => EdgeKind::Mock,
            SpecKind::ReExport => EdgeKind::ReExport,
            SpecKind::TypeReExport => EdgeKind::TypeReExport,
        }
    }
}

/// One step of a path: the file, and the edge kind that led to it.
pub type Step = (String, Option<EdgeKind>);

#[derive(Debug, Default)]
pub struct Graph {
    pub files: Vec<String>,
    index: HashMap<String, u32>,
    deps: Vec<Vec<(u32, EdgeKind)>>,
    rdeps: Vec<Vec<(u32, EdgeKind)>>,
    /// Package names, and for each file the package it sits in.
    pub packages: Vec<String>,
    package_of: Vec<Option<u32>>,
    /// Files that depend on a whole package, by package.
    package_importers: Vec<Vec<u32>>,
    /// Unresolved specifiers, by file.
    pub unresolved: Vec<(u32, String)>,
    /// Every specifier that didn't resolve, uninstalled packages included.
    pub failed: Vec<(u32, String, EdgeKind)>,
    /// Packages from outside the repository each file imports, by name:
    /// what a dependency's version bump reaches.
    pub external: Vec<(u32, String)>,
    /// Path-like literals naming no file in the tree, as repo paths.
    pub dangling: Vec<(u32, String)>,
    /// Package folders, parallel to `packages`.
    package_dirs: Vec<String>,
    /// Files with a dynamic import whose target isn't a plain string.
    pub unknown: Vec<u32>,
    /// Files whose tsconfig couldn't be applied.
    pub tsconfig_fallbacks: Vec<u32>,
    /// Files the walk reaches but doesn't go past (`graph.barrier`).
    pub barrier: std::collections::HashSet<u32>,
    /// Import edges added for a named import resolved by name, which a
    /// file reached only through its re-exports doesn't follow.
    named: HashSet<(u32, u32, EdgeKind)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stats {
    pub files: usize,
    pub edges: usize,
    pub edges_by_kind: Vec<(EdgeKind, usize)>,
    pub package_edges: usize,
    pub packages: usize,
    pub unresolved: usize,
    pub unknown: usize,
    pub tsconfig_fallbacks: usize,
}

impl Graph {
    pub fn with_files(files: Vec<String>, packages: Vec<(String, String)>) -> Graph {
        let index = files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.clone(), i as u32))
            .collect();
        let dirs: Vec<String> = packages.iter().map(|(_, dir)| format!("{dir}/")).collect();
        let package_of = files
            .iter()
            .map(|f| {
                dirs.iter()
                    .enumerate()
                    .filter(|(_, dir)| f.starts_with(dir.as_str()))
                    .max_by_key(|(_, dir)| dir.len())
                    .map(|(i, _)| i as u32)
            })
            .collect();
        let n = files.len();
        Graph {
            files,
            index,
            deps: vec![Vec::new(); n],
            rdeps: vec![Vec::new(); n],
            package_importers: vec![Vec::new(); packages.len()],
            package_dirs: packages.iter().map(|(_, dir)| format!("{dir}/")).collect(),
            packages: packages.into_iter().map(|(name, _)| name).collect(),
            package_of,
            ..Graph::default()
        }
    }

    /// Adds a file that isn't in the tree, such as one a change deleted, so
    /// edges can point at it; returns its id, or the existing one.
    pub fn add_phantom(&mut self, file: &str) -> u32 {
        if let Some(id) = self.id(file) {
            return id;
        }
        let id = self.files.len() as u32;
        self.files.push(file.to_string());
        self.index.insert(file.to_string(), id);
        self.deps.push(Vec::new());
        self.rdeps.push(Vec::new());
        let package = self
            .package_dirs
            .iter()
            .enumerate()
            .filter(|(_, dir)| file.starts_with(dir.as_str()))
            .max_by_key(|(_, dir)| dir.len())
            .map(|(i, _)| i as u32);
        self.package_of.push(package);
        id
    }

    pub fn id(&self, file: &str) -> Option<u32> {
        self.index.get(file).copied()
    }

    pub fn add_edge(&mut self, from: u32, to: u32, kind: EdgeKind) {
        if !self.named.is_empty() {
            self.named.remove(&(from, to, kind));
        }
        if from != to && !self.deps[from as usize].contains(&(to, kind)) {
            self.deps[from as usize].push((to, kind));
            self.rdeps[to as usize].push((from, kind));
        }
    }

    /// An edge for a named import resolved by name: the importer needs
    /// `to`'s own code, not what `to` re-exports. The same edge added any
    /// other way, before or after, wins.
    pub fn add_named_edge(&mut self, from: u32, to: u32, kind: EdgeKind) {
        if from == to || self.deps[from as usize].contains(&(to, kind)) {
            return;
        }
        self.named.insert((from, to, kind));
        self.deps[from as usize].push((to, kind));
        self.rdeps[to as usize].push((from, kind));
    }

    /// Whether `from`'s edge to `to` was a named import resolved by name.
    pub fn is_named(&self, from: u32, to: u32, kind: EdgeKind) -> bool {
        self.named.contains(&(from, to, kind))
    }

    /// The importers a walk goes on to from `file`, each with whether it's
    /// reached whole, which only a re-export edge doesn't give. `whole`
    /// says how `file` was reached: one reached only over re-exports
    /// isn't followed along a named edge.
    pub fn next(&self, file: u32, whole: bool) -> Vec<(u32, bool)> {
        let mut out: Vec<(u32, bool)> = Vec::new();
        for &(from, kind) in &self.rdeps[file as usize] {
            if !whole && self.named.contains(&(from, file, kind)) {
                continue;
            }
            out.push((from, !kind.is_reexport()));
        }
        if let Some(p) = self.package_of[file as usize] {
            out.extend(
                self.package_importers[p as usize]
                    .iter()
                    .map(|&f| (f, true)),
            );
        }
        out.sort_unstable_by_key(|&(f, w)| (f, !w));
        out.dedup_by_key(|(f, _)| *f);
        out
    }

    /// `add_edge` without the duplicate check, for callers that dedupe a
    /// batch themselves: rule edges run to thousands per file.
    pub fn add_edge_unchecked(&mut self, from: u32, to: u32, kind: EdgeKind) {
        self.deps[from as usize].push((to, kind));
        self.rdeps[to as usize].push((from, kind));
    }

    /// The workspace package a file sits in.
    pub fn package_name_of(&self, id: u32) -> Option<&str> {
        let p = self.package_of.get(id as usize).copied().flatten()?;
        Some(self.packages[p as usize].as_str())
    }

    pub fn is_barrier(&self, id: u32) -> bool {
        self.barrier.contains(&id)
    }

    pub fn add_package_edge(&mut self, from: u32, package: &str) {
        if let Some(p) = self.packages.iter().position(|n| n == package) {
            if !self.package_importers[p].contains(&from) {
                self.package_importers[p].push(from);
            }
        }
    }

    /// The files `id` depends on directly (package edges not included).
    pub fn dependencies(&self, id: u32) -> &[(u32, EdgeKind)] {
        &self.deps[id as usize]
    }

    /// Files that depend on `id` directly, including through a package edge.
    pub fn importers(&self, id: u32) -> Vec<(u32, Option<EdgeKind>)> {
        let mut out: Vec<(u32, Option<EdgeKind>)> = self.rdeps[id as usize]
            .iter()
            .map(|&(f, k)| (f, Some(k)))
            .collect();
        if let Some(p) = self.package_of[id as usize] {
            out.extend(
                self.package_importers[p as usize]
                    .iter()
                    .map(|&f| (f, None)),
            );
        }
        // A direct edge names its kind; keep it over the package edge.
        out.sort_by_key(|&(f, k)| (f, k.is_none(), k));
        out.dedup_by_key(|(f, _)| *f);
        out
    }

    /// The walk from `changed`, in both states a file can be reached in:
    /// whole, and only through re-exports. Each entry names the file and
    /// state it was reached from, which was reached first, so following
    /// them always ends at a changed file.
    fn reach(&self, changed: &[u32]) -> [HashMap<u32, Option<(u32, bool)>>; 2] {
        let mut seen: [HashMap<u32, Option<(u32, bool)>>; 2] = Default::default();
        let mut queue: VecDeque<(u32, bool)> = VecDeque::new();
        for &c in changed {
            if seen[1].insert(c, None).is_none() {
                queue.push_back((c, true));
            }
        }
        while let Some((file, whole)) = queue.pop_front() {
            for (importer, reach_whole) in self.next(file, whole) {
                let known = seen[1].contains_key(&importer)
                    || (!reach_whole && seen[0].contains_key(&importer));
                if !known {
                    seen[reach_whole as usize].insert(importer, Some((file, whole)));
                    queue.push_back((importer, reach_whole));
                }
            }
        }
        seen
    }

    /// Every file that can be affected by a change to any of `changed`, with
    /// the file that led to each one, for reasons.
    pub fn affected(&self, changed: &[u32]) -> HashMap<u32, Option<u32>> {
        let [through, whole] = self.reach(changed);
        let mut out: HashMap<u32, Option<u32>> = through
            .into_iter()
            .map(|(f, via)| (f, via.map(|v| v.0)))
            .collect();
        out.extend(whole.into_iter().map(|(f, via)| (f, via.map(|v| v.0))));
        out
    }

    /// The shortest chain by which `from` depends on `to`, `from` first.
    pub fn why(&self, from: u32, to: u32) -> Option<Vec<Step>> {
        let seen = self.reach(&[to]);
        let mut state = if seen[1].contains_key(&from) {
            true
        } else {
            seen[0].get(&from)?;
            false
        };
        let mut chain = Vec::new();
        let mut at = Some(from);
        while let Some(file) = at {
            let next = seen[state as usize][&file];
            let kind = next.and_then(|(n, n_whole)| {
                self.deps[file as usize]
                    .iter()
                    .filter(|&&(t, k)| t == n && (n_whole || !self.is_named(file, n, k)))
                    .map(|(_, k)| *k)
                    .min()
            });
            chain.push((self.files[file as usize].clone(), kind));
            at = next.map(|(n, s)| {
                state = s;
                n
            });
        }
        Some(chain)
    }

    pub fn stats(&self) -> Stats {
        let mut by_kind: HashMap<EdgeKind, usize> = HashMap::new();
        for edges in &self.deps {
            for (_, kind) in edges {
                *by_kind.entry(*kind).or_default() += 1;
            }
        }
        let mut edges_by_kind: Vec<(EdgeKind, usize)> = by_kind.into_iter().collect();
        edges_by_kind.sort();
        Stats {
            files: self.files.len(),
            edges: self.deps.iter().map(Vec::len).sum(),
            edges_by_kind,
            package_edges: self.package_importers.iter().map(Vec::len).sum(),
            packages: self.packages.len(),
            unresolved: self.unresolved.len(),
            unknown: self.unknown.len(),
            tsconfig_fallbacks: self.tsconfig_fallbacks.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0 imports `a` from barrel 1, which re-exports 2 (declaring `a`) and 3;
    /// 4 imports all of 1; 5 re-exports 1; 6 imports `a` from 5.
    fn barrel() -> Graph {
        let files = (0..7).map(|i| format!("f{i}.ts")).collect();
        let mut g = Graph::with_files(files, Vec::new());
        g.add_edge(1, 2, EdgeKind::ReExport);
        g.add_edge(1, 3, EdgeKind::ReExport);
        g.add_named_edge(0, 1, EdgeKind::Import);
        g.add_named_edge(0, 2, EdgeKind::Import);
        g.add_edge(4, 1, EdgeKind::Import);
        g.add_edge(5, 1, EdgeKind::ReExport);
        for to in [5, 1, 2] {
            g.add_named_edge(6, to, EdgeKind::Import);
        }
        g
    }

    fn reached(g: &Graph, changed: u32) -> Vec<u32> {
        let mut out: Vec<u32> = g.affected(&[changed]).into_keys().collect();
        out.sort();
        out
    }

    #[test]
    fn a_named_import_resolved_past_a_barrel_skips_what_else_it_re_exports() {
        let g = barrel();
        assert_eq!(reached(&g, 3), [1, 3, 4, 5]);
        assert_eq!(reached(&g, 2), [0, 1, 2, 4, 5, 6]);
        // The barrel's own code reaches everything that imports from it.
        assert_eq!(reached(&g, 1), [0, 1, 4, 5, 6]);
        assert_eq!(
            g.why(4, 3).unwrap(),
            vec![
                ("f4.ts".to_string(), Some(EdgeKind::Import)),
                ("f1.ts".to_string(), Some(EdgeKind::ReExport)),
                ("f3.ts".to_string(), None),
            ]
        );
        assert_eq!(g.why(0, 3), None);
    }

    #[test]
    fn the_same_edge_added_any_other_way_is_followed_whole() {
        let mut g = barrel();
        g.add_edge(0, 1, EdgeKind::Import);
        assert_eq!(reached(&g, 3), [0, 1, 3, 4, 5]);
        let mut g = barrel();
        g.add_edge(0, 1, EdgeKind::Mock);
        assert_eq!(reached(&g, 3), [0, 1, 3, 4, 5]);
        // A named edge after a plain one doesn't narrow it.
        let mut g = barrel();
        g.add_edge(6, 5, EdgeKind::TypeImport);
        g.add_named_edge(6, 5, EdgeKind::TypeImport);
        assert_eq!(reached(&g, 3), [1, 3, 4, 5, 6]);
    }

    #[test]
    fn a_file_reached_through_re_exports_then_whole_goes_on_to_named_importers() {
        let mut g = barrel();
        // 1 also imports 3 for its own code.
        g.add_edge(1, 3, EdgeKind::Import);
        assert_eq!(reached(&g, 3), [0, 1, 3, 4, 5, 6]);
        assert_eq!(
            g.why(0, 3).unwrap(),
            vec![
                ("f0.ts".to_string(), Some(EdgeKind::Import)),
                ("f1.ts".to_string(), Some(EdgeKind::Import)),
                ("f3.ts".to_string(), None),
            ]
        );
    }
}
