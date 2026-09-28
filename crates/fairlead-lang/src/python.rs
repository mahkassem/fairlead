//! Python: a file's imports, and how a dotted module name becomes a file.
//! Importing `a.b.c` runs `a/__init__.py` and `a/b/__init__.py` too, so
//! those are edges as well; `from a.b import c` may name the module `a.b.c`
//! or a name defined in `a.b`, and the file decides which. A pytest test
//! depends on every `conftest.py` above it, which pytest loads first.

use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser};

use crate::extract::{looks_like_path, Extracted, SpecKind};
use crate::tree::{normalize, parent, Tree};

/// The Python scanner's id, in reports.
pub const ID: &str = "python";

pub fn language() -> Language {
    tree_sitter_python::LANGUAGE.into()
}

fn text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

fn parser() -> Option<Parser> {
    static LANGUAGE: OnceLock<Language> = OnceLock::new();
    let mut parser = Parser::new();
    parser.set_language(LANGUAGE.get_or_init(language)).ok()?;
    Some(parser)
}

/// The module an `aliased_import` or `dotted_name` names.
fn module_name(node: Node, source: &[u8]) -> Option<String> {
    let name = match node.kind() {
        "aliased_import" => node.child_by_field_name("name")?,
        "dotted_name" => node,
        _ => return None,
    };
    Some(text(name, source).split_whitespace().collect())
}

/// A string's content when it has no interpolation.
fn literal(node: Node, source: &[u8]) -> Option<String> {
    let mut w = node.walk();
    let mut out = String::new();
    for child in node.named_children(&mut w) {
        match child.kind() {
            "string_content" => out.push_str(text(child, source)),
            "string_start" | "string_end" => {}
            _ => return None,
        }
    }
    Some(out)
}

/// Imports as specs: `a.b` for `import a.b`, `a.b:c` for `from a.b import
/// c` (module, then name), with a relative module's leading dots kept.
pub fn extract(source: &[u8]) -> Extracted {
    let mut out = Extracted::default();
    let Some(tree) = parser().and_then(|mut p| p.parse(source, None)) else {
        return out;
    };
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "import_statement" => {
                let mut w = node.walk();
                for name in node.children_by_field_name("name", &mut w) {
                    if let Some(m) = module_name(name, source) {
                        out.specs.push((m, SpecKind::Import));
                    }
                }
                continue;
            }
            "import_from_statement" => {
                let Some(module) = node.child_by_field_name("module_name") else {
                    continue;
                };
                let module: String = text(module, source).split_whitespace().collect();
                let mut w = node.walk();
                let names: Vec<String> = node
                    .children_by_field_name("name", &mut w)
                    .filter_map(|n| module_name(n, source))
                    .collect();
                if names.is_empty() {
                    out.specs.push((format!("{module}:*"), SpecKind::Import));
                }
                for name in names {
                    out.specs
                        .push((format!("{module}:{name}"), SpecKind::Import));
                }
                continue;
            }
            "future_import_statement" => continue,
            "string" => {
                if let Some(s) = literal(node, source).filter(|s| looks_like_path(s)) {
                    out.literals.push(s);
                }
                continue;
            }
            _ => {}
        }
        let mut w = node.walk();
        let children: Vec<Node> = node.children(&mut w).collect();
        stack.extend(children.into_iter().rev());
    }
    out.specs.sort();
    out.specs.dedup();
    out.literals.sort();
    out.literals.dedup();
    out
}

pub fn is_test(file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    (name.starts_with("test_") || name.ends_with("_test.py") || name == "conftest.py")
        && name.ends_with(".py")
}

/// Where top-level modules live: the repository root, each folder holding
/// a `pyproject.toml`, `setup.py` or `setup.cfg`, and the `src` beside
/// any of those.
#[derive(Debug, Default)]
pub struct Roots {
    roots: Vec<String>,
}

impl Roots {
    pub fn new(tree: &Tree) -> Roots {
        let mut roots = vec![String::new()];
        for file in &tree.files {
            let name = file.rsplit('/').next().unwrap_or(file);
            if matches!(name, "pyproject.toml" | "setup.py" | "setup.cfg") {
                roots.push(parent(file).to_string());
            }
        }
        let with_src: Vec<String> = roots
            .iter()
            .map(|r| normalize(r, "src").unwrap_or_else(|| "src".into()))
            .filter(|src| {
                let prefix = format!("{src}/");
                tree.files.iter().any(|f| f.starts_with(&prefix))
            })
            .collect();
        roots.extend(with_src);
        roots.sort();
        roots.dedup();
        Roots { roots }
    }

    /// The roots to try for `file`: the ones it sits under, deepest first,
    /// then the rest.
    fn ordered(&self, file: &str) -> Vec<&str> {
        let under = |r: &str| r.is_empty() || file.starts_with(&format!("{r}/"));
        let mut mine: Vec<&str> = self
            .roots
            .iter()
            .map(String::as_str)
            .filter(|r| under(r))
            .collect();
        mine.sort_by_key(|r| std::cmp::Reverse(r.len()));
        mine.extend(self.roots.iter().map(String::as_str).filter(|r| !under(r)));
        mine
    }

    /// The files an import reaches: the module and the package `__init__.py`
    /// files above it. Standard library and installed packages give none.
    pub fn resolve(&self, tree: &Tree, file: &str, spec: &str) -> Vec<String> {
        let (module, name) = match spec.split_once(':') {
            Some((m, n)) => (m, Some(n).filter(|n| *n != "*")),
            None => (spec, None),
        };
        let dots = module.len() - module.trim_start_matches('.').len();
        let rest = &module[dots..];
        let bases: Vec<String> = if dots > 0 {
            let mut dir = parent(file).to_string();
            for _ in 1..dots {
                dir = parent(&dir).to_string();
            }
            vec![dir]
        } else {
            self.ordered(file).into_iter().map(String::from).collect()
        };
        for base in bases {
            let join = |dotted: &str| -> String {
                let path = dotted.replace('.', "/");
                if base.is_empty() {
                    path
                } else if path.is_empty() {
                    base.clone()
                } else {
                    format!("{base}/{path}")
                }
            };
            let dotted = |extra: Option<&str>| match (rest, extra) {
                ("", Some(n)) => n.to_string(),
                (r, Some(n)) => format!("{r}.{n}"),
                (r, None) => r.to_string(),
            };
            let found = name
                .and_then(|n| module_file(tree, &join(&dotted(Some(n)))))
                .or_else(|| module_file(tree, &join(&dotted(None))));
            if let Some(found) = found {
                let mut out = inits(tree, &base, &found);
                out.push(found);
                return out;
            }
        }
        Vec::new()
    }
}

/// `path.py`, else `path/__init__.py`.
fn module_file(tree: &Tree, path: &str) -> Option<String> {
    let as_file = format!("{path}.py");
    if !path.is_empty() && tree.contains(&as_file) {
        return Some(as_file);
    }
    let init = if path.is_empty() {
        "__init__.py".to_string()
    } else {
        format!("{path}/__init__.py")
    };
    tree.contains(&init).then_some(init)
}

/// Each package `__init__.py` between `base` and `found`.
fn inits(tree: &Tree, base: &str, found: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut dir = parent(found);
    while dir.len() > base.len() {
        let init = format!("{dir}/__init__.py");
        if init != found && tree.contains(&init) {
            out.push(init);
        }
        dir = parent(dir);
    }
    out
}

/// Every `conftest.py` in `file`'s folder and above it.
pub fn conftests(tree: &Tree, file: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut dir = parent(file);
    loop {
        let conftest = normalize(dir, "conftest.py").unwrap_or_else(|| "conftest.py".into());
        if conftest != file && tree.contains(&conftest) {
            out.push(conftest);
        }
        if dir.is_empty() {
            return out;
        }
        dir = parent(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_from_imports_relative_and_wildcards() {
        let src = r#"
from __future__ import annotations
import os, shop.cart as cart
from shop.price import (
    total,
    Rounding as R,
)
from . import helpers
from ..models import *
import importlib

def lazy():
    from shop import late

DATA = "tests/data/basket.json"
NAME = f"tests/{x}.json"
"#;
        let e = extract(src.as_bytes());
        let specs: Vec<&str> = e.specs.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            specs,
            [
                "..models:*",
                ".:helpers",
                "importlib",
                "os",
                "shop.cart",
                "shop.price:Rounding",
                "shop.price:total",
                "shop:late",
            ]
        );
        assert_eq!(e.literals, ["tests/data/basket.json"]);
    }
}
