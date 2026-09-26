//! Modules: the units a plan widens to. Each workspace package is one, and
//! each directory a `modules.define` pattern matches is another. A file
//! belongs to the module with the longest root above it, or to none (the
//! repository root).

use std::collections::BTreeSet;

use fairlead_core::config::{Discover, Modules as ModulesConfig};
use fairlead_lang::tree::{parent, Tree};
use fairlead_lang::workspace::Package;

use crate::pattern::Pattern;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    /// Repo-relative directory, no trailing `/`.
    pub root: String,
}

#[derive(Debug, Clone, Default)]
pub struct Modules {
    list: Vec<Module>,
}

impl Modules {
    pub fn discover(
        tree: &Tree,
        packages: &[Package],
        config: &ModulesConfig,
    ) -> Result<Modules, String> {
        let mut list: Vec<Module> = Vec::new();
        if config.discover.items().contains(&Discover::Workspaces) {
            list.extend(packages.iter().map(|p| Module {
                name: p.name.clone(),
                root: p.dir.clone(),
            }));
        }
        let dirs = directories(tree);
        for def in config.define.items() {
            let pattern = Pattern::new(&def.pattern)?;
            for dir in &dirs {
                if let Some(caps) = pattern.captures(dir) {
                    let name = caps.get("name").cloned().unwrap_or_else(|| dir.clone());
                    list.push(Module {
                        name,
                        root: dir.clone(),
                    });
                }
            }
        }
        let mut seen = BTreeSet::new();
        list.retain(|m| seen.insert(m.root.clone()));
        list.sort_by(|a, b| a.root.cmp(&b.root));
        Ok(Modules { list })
    }

    pub fn from_list(list: Vec<Module>) -> Modules {
        Modules { list }
    }

    pub fn all(&self) -> &[Module] {
        &self.list
    }

    pub fn get(&self, index: usize) -> &Module {
        &self.list[index]
    }

    /// The module `path` belongs to, by index.
    pub fn of(&self, path: &str) -> Option<usize> {
        self.list
            .iter()
            .enumerate()
            .filter(|(_, m)| path.starts_with(&m.root) && path[m.root.len()..].starts_with('/'))
            .max_by_key(|(_, m)| m.root.len())
            .map(|(i, _)| i)
    }

    pub fn name_of(&self, path: &str) -> Option<&str> {
        self.of(path).map(|i| self.list[i].name.as_str())
    }
}

/// Every directory that holds a file, at any depth.
fn directories(tree: &Tree) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    for file in &tree.files {
        let mut dir = parent(file);
        while !dir.is_empty() && dirs.insert(dir.to_string()) {
            dir = parent(dir);
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules(roots: &[&str]) -> Modules {
        Modules::from_list(
            roots
                .iter()
                .map(|r| Module {
                    name: r.to_string(),
                    root: r.to_string(),
                })
                .collect(),
        )
    }

    #[test]
    fn a_file_belongs_to_the_deepest_module_above_it() {
        let m = modules(&["packages/a", "packages/a/sub", "packages/ab"]);
        assert_eq!(m.name_of("packages/a/x.ts"), Some("packages/a"));
        assert_eq!(m.name_of("packages/a/sub/x.ts"), Some("packages/a/sub"));
        assert_eq!(m.name_of("packages/ab/x.ts"), Some("packages/ab"));
        assert_eq!(m.name_of("packages/x.ts"), None);
    }
}
