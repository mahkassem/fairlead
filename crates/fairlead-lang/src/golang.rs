//! Go: a file's imports and `//go:embed` patterns, and how `go.mod` turns an
//! import path into a package directory. A package is every `.go` file in
//! its directory, so importing it depends on each of them, and a test file
//! depends on every other file of its directory, which it's compiled with.

use std::collections::HashMap;

use fairlead_core::pattern::Pattern;

use crate::extract::{Extracted, SpecKind};
use crate::tree::{normalize, parent, Tree};

/// The Go scanner's id, in reports.
pub const ID: &str = "go";

pub fn is_test(file: &str) -> bool {
    file.ends_with("_test.go")
}

/// Skips spaces, newlines and comments.
fn skip(s: &str) -> &str {
    let mut s = s;
    loop {
        let t = s.trim_start();
        if let Some(rest) = t.strip_prefix("//") {
            s = rest.split_once('\n').map_or("", |(_, r)| r);
        } else if let Some(rest) = t.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r);
        } else {
            return t;
        }
    }
}

fn word(s: &str) -> (&str, &str) {
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

/// A `"…"` or `` `…` `` string at the start of `s`, and what follows it.
fn string(s: &str) -> Option<(String, &str)> {
    let quote = s.chars().next().filter(|c| *c == '"' || *c == '`')?;
    let end = s[1..].find(quote)? + 1;
    Some((s[1..end].to_string(), &s[end + 1..]))
}

/// One import spec, `[name] "path"`, and what follows it.
fn spec(s: &str) -> Option<(String, &str)> {
    let s = skip(s);
    let s = match s.chars().next()? {
        '"' | '`' => s,
        _ => skip(word(s).1),
    };
    string(s)
}

/// The import paths, read from the declarations after the package clause
/// until the first thing that isn't an import.
fn imports(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let s = skip(text);
    let Some(s) = s.strip_prefix("package") else {
        return out;
    };
    let mut s = skip(word(skip(s)).1);
    loop {
        s = skip(s.trim_start_matches(';'));
        let (keyword, rest) = word(s);
        if keyword != "import" {
            return out;
        }
        let rest = skip(rest);
        if let Some(mut group) = rest.strip_prefix('(') {
            loop {
                group = skip(group.trim_start_matches(';'));
                if let Some(after) = group.strip_prefix(')') {
                    s = after;
                    break;
                }
                let Some((path, after)) = spec(group) else {
                    return out;
                };
                out.push(path);
                group = after;
            }
        } else {
            let Some((path, after)) = spec(rest) else {
                return out;
            };
            out.push(path);
            s = after;
        }
    }
}

/// Each `//go:embed` pattern, quoted or not.
fn embeds(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(mut rest) = line.trim_start().strip_prefix("//go:embed ") else {
            continue;
        };
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                break;
            }
            if let Some((s, after)) = string(rest) {
                out.push(s);
                rest = after;
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                out.push(rest[..end].to_string());
                rest = &rest[end..];
            }
        }
    }
    out
}

/// Imports as specs of kind `Import`, embed patterns of kind `Require`.
pub fn extract(source: &[u8]) -> Extracted {
    let text = String::from_utf8_lossy(source);
    let mut specs: Vec<(String, SpecKind)> = imports(&text)
        .into_iter()
        .map(|p| (p, SpecKind::Import))
        .chain(embeds(&text).into_iter().map(|p| (p, SpecKind::Require)))
        .collect();
    specs.sort();
    specs.dedup();
    Extracted {
        specs,
        ..Extracted::default()
    }
}

/// The modules in the tree, from every `go.mod` (so a `go.work` workspace's
/// modules are all found) with their local `replace` directives, and each
/// directory's Go files.
#[derive(Debug, Default)]
pub struct Modules {
    /// Module path and its directory, longest path first.
    modules: Vec<(String, String)>,
    /// Each `go.mod`'s directory, deepest first, for `vendor/`.
    roots: Vec<String>,
    dirs: HashMap<String, Vec<String>>,
}

impl Modules {
    pub fn new(tree: &Tree) -> Modules {
        let mut modules = Vec::new();
        let mut roots = Vec::new();
        for manifest in tree
            .files
            .iter()
            .filter(|f| *f == "go.mod" || f.ends_with("/go.mod"))
        {
            roots.push(parent(manifest).to_string());
            let Ok(text) = std::fs::read_to_string(tree.abs(manifest)) else {
                continue;
            };
            modules.extend(read_mod(parent(manifest), &text));
        }
        modules.sort_by(|a: &(String, String), b| b.0.len().cmp(&a.0.len()).then_with(|| a.cmp(b)));
        modules.dedup();
        let mut dirs: HashMap<String, Vec<String>> = HashMap::new();
        for file in tree.files.iter().filter(|f| f.ends_with(".go")) {
            dirs.entry(parent(file).to_string())
                .or_default()
                .push(file.clone());
        }
        roots.sort_by_key(|r| std::cmp::Reverse(r.len()));
        Modules {
            modules,
            roots,
            dirs,
        }
    }

    /// The files of the package an import path names: through the module
    /// that owns it, else under the importing module's `vendor/`. Standard
    /// library and modules outside the tree give none.
    pub fn package(&self, file: &str, import: &str) -> Vec<String> {
        for (module, dir) in &self.modules {
            let rest = if import == module {
                Some("")
            } else {
                import
                    .strip_prefix(module.as_str())
                    .and_then(|r| r.strip_prefix('/'))
            };
            if let Some(rest) = rest {
                let target = normalize(dir, rest).unwrap_or_else(|| dir.clone());
                return self.sources(&target);
            }
        }
        let root = self
            .roots
            .iter()
            .find(|r| r.is_empty() || file.starts_with(&format!("{r}/")));
        match root {
            Some(root) => {
                self.sources(&normalize(root, &format!("vendor/{import}")).unwrap_or_default())
            }
            None => Vec::new(),
        }
    }

    fn sources(&self, dir: &str) -> Vec<String> {
        self.dirs
            .get(dir)
            .map(|files| files.iter().filter(|f| !is_test(f)).cloned().collect())
            .unwrap_or_default()
    }

    /// The `go.mod` and `go.sum` of the module `file` belongs to.
    pub fn manifests(&self, tree: &Tree, file: &str) -> Vec<String> {
        let Some(root) = self
            .roots
            .iter()
            .find(|r| r.is_empty() || file.starts_with(&format!("{r}/")))
        else {
            return Vec::new();
        };
        ["go.mod", "go.sum"]
            .into_iter()
            .map(|name| normalize(root, name).unwrap_or_else(|| name.to_string()))
            .filter(|path| tree.contains(path))
            .collect()
    }

    /// Every other Go file in `file`'s directory.
    pub fn siblings(&self, file: &str) -> Vec<String> {
        self.dirs
            .get(parent(file))
            .map(|files| files.iter().filter(|f| *f != file).cloned().collect())
            .unwrap_or_default()
    }

    /// The files an embed pattern names, relative to `file`'s directory: a
    /// file, every file under a directory, or a glob.
    pub fn embedded(&self, tree: &Tree, file: &str, pattern: &str) -> Vec<String> {
        let dir = parent(file);
        let pattern = pattern.trim_start_matches("all:");
        let Some(target) = normalize(dir, pattern) else {
            return Vec::new();
        };
        if pattern.contains(['*', '?', '[']) {
            let Ok(glob) = Pattern::new(&target) else {
                return Vec::new();
            };
            return tree
                .files
                .iter()
                .filter(|f| glob.is_match(f))
                .cloned()
                .collect();
        }
        if tree.contains(&target) {
            return vec![target];
        }
        let prefix = format!("{target}/");
        tree.files
            .iter()
            .filter(|f| f.starts_with(&prefix))
            .cloned()
            .collect()
    }
}

/// `module` and each local `replace`, single or in a block.
fn read_mod(dir: &str, text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_replace = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if let Some(path) = line.strip_prefix("module ") {
            out.push((path.trim().trim_matches('"').to_string(), dir.to_string()));
            continue;
        }
        let directive = if in_replace {
            if line == ")" {
                in_replace = false;
                continue;
            }
            line
        } else if line == "replace (" {
            in_replace = true;
            continue;
        } else if let Some(rest) = line.strip_prefix("replace ") {
            rest
        } else {
            continue;
        };
        let Some((from, to)) = directive.split_once("=>") else {
            continue;
        };
        let from = from.split_whitespace().next().unwrap_or("");
        let to = to.trim();
        if !(to.starts_with("./") || to.starts_with("../")) {
            continue;
        }
        if let Some(path) = normalize(dir, to) {
            out.push((from.to_string(), path));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_single_grouped_named_and_commented_until_the_first_declaration() {
        let src = r#"// Package calc adds.
package calc // trailing

import "fmt"
import (
	m "math"
	_ "embed" /* blank */
	. "strings"
	`example.com/calc/internal/raw`
)

//go:embed templates static/*.css "with space.txt"
var files embed.FS

import "never/after/a/declaration"
"#;
        let e = extract(src.as_bytes());
        let of = |k: SpecKind| -> Vec<&str> {
            e.specs
                .iter()
                .filter(|(_, x)| *x == k)
                .map(|(s, _)| s.as_str())
                .collect()
        };
        assert_eq!(
            of(SpecKind::Import),
            [
                "embed",
                "example.com/calc/internal/raw",
                "fmt",
                "math",
                "strings"
            ]
        );
        assert_eq!(
            of(SpecKind::Require),
            ["static/*.css", "templates", "with space.txt"]
        );
    }

    #[test]
    fn go_mod_gives_the_module_and_its_local_replacements() {
        let text = "module example.com/app\n\ngo 1.24\n\nrequire example.com/lib v1.0.0\n\nreplace example.com/lib => ../lib\nreplace (\n\texample.com/tools v1.2.0 => ./tools // local\n\texample.com/far => example.com/fork v1.0.0\n)\n";
        assert_eq!(
            read_mod("services/app", text),
            [
                ("example.com/app".to_string(), "services/app".to_string()),
                ("example.com/lib".to_string(), "services/lib".to_string()),
                (
                    "example.com/tools".to_string(),
                    "services/app/tools".to_string()
                ),
            ]
        );
    }
}
