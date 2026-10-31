//! Following a named import through barrels: `import { a } from "./lib"`,
//! where `lib/index.ts` re-exports `a` from `./a`, depends on `lib/a.ts`
//! for `a`, and on each barrel on the way for its own code only. Whatever
//! can't be followed by name keeps the importer on the whole module.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::exports::Shape;

/// How many re-exports a name is followed through before giving up on it.
pub const MAX_DEPTH: usize = 32;

/// A module's names with each re-export's target resolved to a file, or
/// `None` when it isn't a file in the repository (a package, an external
/// module, one that didn't resolve, or a type-only one left out).
#[derive(Debug, Default, Clone)]
pub struct Module {
    pub local: HashSet<String>,
    /// Exported name, the name in the target (`*` for a namespace), target.
    pub named: Vec<(String, String, Option<u32>)>,
    pub stars: Vec<Option<u32>>,
    pub opaque: bool,
    /// The module does something when it loads.
    pub effects: bool,
    /// The files its value re-exports load, which a type-only one doesn't.
    pub loads: Vec<u32>,
}

impl Module {
    /// `shape` with its specifiers resolved by `target`.
    pub fn new(shape: &Shape, target: impl Fn(&str, bool) -> Option<u32>) -> Module {
        let mut module = Module {
            local: shape.local.iter().cloned().collect(),
            opaque: shape.opaque,
            effects: shape.effects,
            ..Module::default()
        };
        for r in &shape.reexports {
            let to = target(&r.spec, r.type_only);
            if let (Some(to), false) = (to, r.type_only) {
                module.loads.push(to);
            }
            match &r.names {
                None => module.stars.push(to),
                Some(names) => module.named.extend(
                    names
                        .iter()
                        .map(|(exported, original)| (exported.clone(), original.clone(), to)),
                ),
            }
        }
        module
    }
}

/// Where a name led: the files it depends on, each marked whole when the
/// importer needs all of it (a namespace re-export), or why it can't say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Files(Vec<(u32, bool)>),
    /// The module visibly doesn't export the name.
    Missing,
    /// It might, through something that can't be read by name.
    Unknown,
}

/// Every module's names, by file id, and the lookups made so far.
#[derive(Debug, Default)]
pub struct Barrels {
    modules: HashMap<u32, Module>,
    /// Modules that re-export, directly or through other barrels, one that
    /// does something when it loads.
    effectful: HashSet<u32>,
    memo: Mutex<HashMap<(u32, String), Found>>,
}

impl Barrels {
    pub fn new(modules: HashMap<u32, Module>) -> Barrels {
        Barrels {
            effectful: effectful(&modules),
            modules,
            memo: Mutex::default(),
        }
    }

    /// The files `name`, imported from `file`, depends on: `file` first,
    /// then each barrel on the way, then the file that declares it.
    /// Importing a name runs every module behind the barrel, so one with
    /// an effect among them keeps the importer on the whole barrel.
    pub fn lookup(&self, file: u32, name: &str) -> Found {
        if self.effectful.contains(&file) {
            return Found::Unknown;
        }
        self.find(file, name, 0, &mut HashSet::new()).0
    }

    /// The lookup, and whether a cycle cut it short, which makes the answer
    /// hold only for this entry point.
    fn find(
        &self,
        file: u32,
        name: &str,
        depth: usize,
        visiting: &mut HashSet<(u32, String)>,
    ) -> (Found, bool) {
        if depth > MAX_DEPTH {
            return (Found::Unknown, true);
        }
        let Some(module) = self.modules.get(&file) else {
            return (Found::Unknown, false);
        };
        if module.local.contains(name) {
            return (Found::Files(vec![(file, false)]), false);
        }
        let key = (file, name.to_string());
        if let Some(found) = self.memo.lock().expect("memo").get(&key) {
            return (found.clone(), false);
        }
        if !visiting.insert(key.clone()) {
            // A cycle: the name isn't found by going round again.
            return (Found::Missing, true);
        }
        let (found, cut) = self.through(module, file, name, depth, visiting);
        visiting.remove(&key);
        // An answer a cycle cut short, or the depth bound, holds only for
        // this way in.
        if !cut {
            self.memo.lock().expect("memo").insert(key, found.clone());
        }
        (found, cut)
    }

    /// A name that isn't declared in `module`: through its named
    /// re-exports, then its `export *`s.
    fn through(
        &self,
        module: &Module,
        file: u32,
        name: &str,
        depth: usize,
        visiting: &mut HashSet<(u32, String)>,
    ) -> (Found, bool) {
        let mut cut = false;
        let mut files = vec![(file, false)];
        let named: Vec<_> = module.named.iter().filter(|(e, _, _)| e == name).collect();
        if !named.is_empty() {
            for (_, original, to) in named {
                let Some(to) = *to else {
                    return (Found::Unknown, cut);
                };
                if original == "*" {
                    files.push((to, true));
                    continue;
                }
                let (found, c) = self.find(to, original, depth + 1, visiting);
                cut |= c;
                match found {
                    Found::Files(more) => files.extend(more),
                    // A re-export of a name its module lacks is broken,
                    // which only the whole module can show.
                    Found::Missing | Found::Unknown => return (Found::Unknown, cut),
                }
            }
            return (Found::Files(files), cut);
        }
        let mut unknown = module.opaque;
        // `export *` never re-exports a default.
        if name != "default" {
            for to in &module.stars {
                let Some(to) = *to else {
                    unknown = true;
                    continue;
                };
                let (found, c) = self.find(to, name, depth + 1, visiting);
                cut |= c;
                match found {
                    Found::Files(more) => files.extend(more),
                    Found::Unknown => unknown = true,
                    Found::Missing => {}
                }
            }
        }
        if files.len() > 1 {
            // Two `export *`s giving the same name make it ambiguous, so
            // every one found counts.
            files.sort_unstable();
            files.dedup();
            (Found::Files(files), cut)
        } else if unknown {
            (Found::Unknown, cut)
        } else {
            (Found::Missing, cut)
        }
    }
}

/// Each module with an effect marks the modules that re-export it, and
/// those the modules that re-export them: one pass backwards, so a cycle
/// ends where it was already marked.
fn effectful(modules: &HashMap<u32, Module>) -> HashSet<u32> {
    let mut by: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&from, module) in modules {
        for &to in &module.loads {
            by.entry(to).or_default().push(from);
        }
    }
    let mut marked = HashSet::new();
    let mut queue: Vec<u32> = modules
        .iter()
        .filter(|(_, m)| m.effects)
        .map(|(&id, _)| id)
        .collect();
    while let Some(at) = queue.pop() {
        for &from in by.get(&at).into_iter().flatten() {
            if marked.insert(from) {
                queue.push(from);
            }
        }
    }
    marked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(local: &[&str]) -> Module {
        Module {
            local: local.iter().map(|n| n.to_string()).collect(),
            ..Module::default()
        }
    }

    fn named(exported: &str, original: &str, to: Option<u32>) -> (String, String, Option<u32>) {
        (exported.into(), original.into(), to)
    }

    fn files(found: &[u32]) -> Found {
        Found::Files(found.iter().map(|&f| (f, false)).collect())
    }

    #[test]
    fn a_name_is_followed_through_a_chain_of_star_re_exports() {
        // 0: export * from 1; 1: export * from 2, 3; 2 declares a; 3 declares b.
        let barrels = Barrels::new(HashMap::from([
            (
                0,
                Module {
                    stars: vec![Some(1)],
                    ..module(&["own"])
                },
            ),
            (
                1,
                Module {
                    stars: vec![Some(2), Some(3)],
                    ..module(&[])
                },
            ),
            (2, module(&["a"])),
            (3, module(&["b"])),
        ]));
        assert_eq!(barrels.lookup(0, "a"), files(&[0, 1, 2]));
        assert_eq!(barrels.lookup(0, "b"), files(&[0, 1, 3]));
        assert_eq!(barrels.lookup(0, "own"), files(&[0]));
        assert_eq!(barrels.lookup(0, "nope"), Found::Missing);
        assert_eq!(barrels.lookup(0, "default"), Found::Missing);
    }

    #[test]
    fn a_renamed_re_export_is_looked_up_by_its_original_name() {
        let barrels = Barrels::new(HashMap::from([
            (
                0,
                Module {
                    named: vec![named("b", "a", Some(1)), named("C", "default", Some(1))],
                    ..module(&[])
                },
            ),
            (1, module(&["a", "default"])),
        ]));
        assert_eq!(barrels.lookup(0, "b"), files(&[0, 1]));
        assert_eq!(barrels.lookup(0, "C"), files(&[0, 1]));
        assert_eq!(barrels.lookup(0, "a"), Found::Missing);
    }

    #[test]
    fn a_namespace_re_export_needs_its_whole_module() {
        let barrels = Barrels::new(HashMap::from([
            (
                0,
                Module {
                    named: vec![named("ns", "*", Some(1))],
                    ..module(&[])
                },
            ),
            (1, module(&["a"])),
        ]));
        assert_eq!(
            barrels.lookup(0, "ns"),
            Found::Files(vec![(0, false), (1, true)])
        );
    }

    #[test]
    fn what_cant_be_read_by_name_is_unknown() {
        let barrels = Barrels::new(HashMap::from([
            // A named re-export from outside the repository.
            (
                0,
                Module {
                    named: vec![named("ext", "ext", None)],
                    ..module(&[])
                },
            ),
            // An `export *` of a module that didn't resolve, beside one that did.
            (
                1,
                Module {
                    stars: vec![None, Some(3)],
                    ..module(&[])
                },
            ),
            // A re-export of a name its module lacks.
            (
                2,
                Module {
                    named: vec![named("gone", "gone", Some(3))],
                    ..module(&[])
                },
            ),
            (3, module(&["here"])),
            (
                4,
                Module {
                    opaque: true,
                    ..module(&["x"])
                },
            ),
            // A file with no shape: scanned lexically, or not JavaScript.
            (
                5,
                Module {
                    stars: vec![Some(6)],
                    ..module(&[])
                },
            ),
        ]));
        assert_eq!(barrels.lookup(0, "ext"), Found::Unknown);
        assert_eq!(barrels.lookup(1, "elsewhere"), Found::Unknown);
        assert_eq!(barrels.lookup(1, "here"), files(&[1, 3]));
        assert_eq!(barrels.lookup(2, "gone"), Found::Unknown);
        assert_eq!(barrels.lookup(4, "x"), files(&[4]));
        assert_eq!(barrels.lookup(4, "y"), Found::Unknown);
        assert_eq!(barrels.lookup(5, "z"), Found::Unknown);
    }

    #[test]
    fn a_cycle_of_re_exports_ends_and_still_finds_what_it_holds() {
        let barrels = Barrels::new(HashMap::from([
            (
                0,
                Module {
                    stars: vec![Some(1)],
                    ..module(&[])
                },
            ),
            (
                1,
                Module {
                    stars: vec![Some(0), Some(2)],
                    ..module(&[])
                },
            ),
            (2, module(&["a"])),
        ]));
        assert_eq!(barrels.lookup(0, "a"), files(&[0, 1, 2]));
        assert_eq!(barrels.lookup(1, "a"), files(&[1, 2]));
        assert_eq!(barrels.lookup(0, "nope"), Found::Missing);
        assert_eq!(barrels.lookup(1, "nope"), Found::Missing);
    }

    #[test]
    fn a_chain_past_the_depth_bound_is_unknown() {
        let deep = MAX_DEPTH as u32 + 2;
        let mut modules: HashMap<u32, Module> = (0..deep)
            .map(|i| {
                (
                    i,
                    Module {
                        stars: vec![Some(i + 1)],
                        ..module(&[])
                    },
                )
            })
            .collect();
        modules.insert(deep, module(&["a"]));
        let barrels = Barrels::new(modules);
        assert_eq!(barrels.lookup(0, "a"), Found::Unknown);
        let Found::Files(found) = barrels.lookup(5, "a") else {
            panic!("a chain within the bound is followed");
        };
        assert_eq!(found.len() as u32, deep - 4);
    }

    #[test]
    fn the_same_name_from_two_star_re_exports_depends_on_both() {
        let barrels = Barrels::new(HashMap::from([
            (
                0,
                Module {
                    stars: vec![Some(1), Some(2)],
                    ..module(&[])
                },
            ),
            (1, module(&["a"])),
            (2, module(&["a"])),
        ]));
        assert_eq!(barrels.lookup(0, "a"), files(&[0, 1, 2]));
    }

    fn star_of(to: &[u32], effects: bool) -> Module {
        Module {
            stars: to.iter().map(|&t| Some(t)).collect(),
            loads: to.to_vec(),
            effects,
            ..module(&[])
        }
    }

    #[test]
    fn a_barrel_with_an_effect_behind_it_keeps_its_importers_whole() {
        // 0 -> 1 -> 2 (effect), 3 declares a; 4 -> 5 has no effect; 6 <-> 7
        // cycle with 8 (effect) behind 7; 9 is an effect that re-exports 3.
        let barrels = Barrels::new(HashMap::from([
            (0, star_of(&[1, 3], false)),
            (1, star_of(&[2], false)),
            (
                2,
                Module {
                    effects: true,
                    ..module(&["b"])
                },
            ),
            (3, module(&["a"])),
            (4, star_of(&[5], false)),
            (5, module(&["a"])),
            (6, star_of(&[7, 3], false)),
            (7, star_of(&[6, 8], false)),
            (
                8,
                Module {
                    effects: true,
                    ..module(&[])
                },
            ),
            (9, star_of(&[3], true)),
        ]));
        assert_eq!(barrels.lookup(0, "a"), Found::Unknown);
        assert_eq!(barrels.lookup(1, "b"), Found::Unknown);
        assert_eq!(barrels.lookup(4, "a"), files(&[4, 5]));
        assert_eq!(barrels.lookup(6, "a"), Found::Unknown);
        assert_eq!(barrels.lookup(7, "a"), Found::Unknown);
        // Its own effect is its own code, which the importer depends on.
        assert_eq!(barrels.lookup(9, "a"), files(&[3, 9]));
        assert_eq!(barrels.lookup(2, "b"), files(&[2]));
    }

    #[test]
    fn a_type_only_re_export_loads_nothing() {
        let barrels = Barrels::new(HashMap::from([
            (
                0,
                Module {
                    stars: vec![Some(1), Some(2)],
                    loads: vec![1],
                    ..module(&[])
                },
            ),
            (1, module(&["a"])),
            (
                2,
                Module {
                    effects: true,
                    ..module(&["T"])
                },
            ),
        ]));
        assert_eq!(barrels.lookup(0, "a"), files(&[0, 1]));
    }
}
