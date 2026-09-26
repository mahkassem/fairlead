//! The file graph. An edge from A to B means A depends on B, so walking
//! edges backwards from a changed file finds everything it can affect. A
//! package edge stands for an edge to every file in that package.

use std::collections::{HashMap, VecDeque};

use crate::extract::SpecKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKind {
    Import,
    TypeImport,
    Dynamic,
    Require,
    Mock,
    /// A string literal naming a file in the repository.
    PathLiteral,
    /// `__snapshots__/<test>.snap` to its test.
    Snapshot,
}

impl From<SpecKind> for EdgeKind {
    fn from(kind: SpecKind) -> Self {
        match kind {
            SpecKind::Import => EdgeKind::Import,
            SpecKind::TypeImport => EdgeKind::TypeImport,
            SpecKind::Dynamic => EdgeKind::Dynamic,
            SpecKind::Require => EdgeKind::Require,
            SpecKind::Mock => EdgeKind::Mock,
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
    /// Files with a dynamic import whose target isn't a plain string.
    pub unknown: Vec<u32>,
    /// Files whose tsconfig couldn't be applied.
    pub tsconfig_fallbacks: Vec<u32>,
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
        let package_of = files
            .iter()
            .map(|f| {
                packages
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, dir))| f.starts_with(&format!("{dir}/")))
                    .max_by_key(|(_, (_, dir))| dir.len())
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
            packages: packages.into_iter().map(|(name, _)| name).collect(),
            package_of,
            ..Graph::default()
        }
    }

    pub fn id(&self, file: &str) -> Option<u32> {
        self.index.get(file).copied()
    }

    pub fn add_edge(&mut self, from: u32, to: u32, kind: EdgeKind) {
        if from != to && !self.deps[from as usize].contains(&(to, kind)) {
            self.deps[from as usize].push((to, kind));
            self.rdeps[to as usize].push((from, kind));
        }
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
        out.sort();
        out.dedup_by_key(|(f, _)| *f);
        out
    }

    /// Every file that can be affected by a change to any of `changed`, with
    /// the file that led to each one, for reasons.
    pub fn affected(&self, changed: &[u32]) -> HashMap<u32, Option<u32>> {
        let mut seen: HashMap<u32, Option<u32>> = changed.iter().map(|&c| (c, None)).collect();
        let mut queue: VecDeque<u32> = changed.iter().copied().collect();
        while let Some(file) = queue.pop_front() {
            for (importer, _) in self.importers(file) {
                if let std::collections::hash_map::Entry::Vacant(slot) = seen.entry(importer) {
                    slot.insert(Some(file));
                    queue.push_back(importer);
                }
            }
        }
        seen
    }

    /// The shortest chain by which `from` depends on `to`, `from` first.
    pub fn why(&self, from: u32, to: u32) -> Option<Vec<Step>> {
        let affected = self.affected(&[to]);
        affected.get(&from)?;
        let mut chain = Vec::new();
        let mut at = Some(from);
        while let Some(file) = at {
            let next = affected[&file];
            let kind = next.and_then(|n| {
                self.deps[file as usize]
                    .iter()
                    .find(|(t, _)| *t == n)
                    .map(|(_, k)| *k)
            });
            chain.push((self.files[file as usize].clone(), kind));
            at = next;
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
