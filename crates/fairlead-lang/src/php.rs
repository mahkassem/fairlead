//! PHP: what a file declares and refers to, and how composer's autoloading
//! turns a class name into a file. Names are resolved against the file's
//! namespace and `use` imports while parsing, so a reference is always a
//! fully qualified name; which file declares it is decided once every file
//! is read.

use std::collections::HashMap;
use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser};

use crate::extract::{looks_like_path, Extracted, SpecKind};
use crate::tree::{normalize, parent, Tree};

/// The PHP scanner's id, in reports.
pub const ID: &str = "php";

pub fn language() -> Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}

/// Names that look like classes in a type or a scope but never name a file.
const RESERVED: [&str; 19] = [
    "self", "static", "parent", "true", "false", "null", "mixed", "void", "never", "iterable",
    "object", "callable", "array", "int", "float", "bool", "string", "__dir__", "__file__",
];

#[derive(Default)]
struct Scope {
    namespace: String,
    classes: HashMap<String, String>,
    functions: HashMap<String, String>,
}

impl Scope {
    fn qualify(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}\\{name}", self.namespace)
        }
    }

    /// A class name as PHP resolves it: fully qualified, `namespace\`,
    /// through an import by its first segment, or in the current namespace.
    fn class(&self, text: &str) -> Option<String> {
        if let Some(full) = text.strip_prefix('\\') {
            return Some(full.to_string());
        }
        if let Some(rest) = text.strip_prefix("namespace\\") {
            return Some(self.qualify(rest));
        }
        let (first, rest) = match text.split_once('\\') {
            Some((f, r)) => (f, Some(r)),
            None => (text, None),
        };
        if rest.is_none() && RESERVED.contains(&first.to_ascii_lowercase().as_str()) {
            return None;
        }
        match (self.classes.get(&first.to_ascii_lowercase()), rest) {
            (Some(full), Some(rest)) => Some(format!("{full}\\{rest}")),
            (Some(full), None) => Some(full.clone()),
            (None, _) => Some(self.qualify(text)),
        }
    }

    /// A called function's candidates: an unqualified call falls back to
    /// the global function when the namespace has none.
    fn function(&self, text: &str) -> Vec<String> {
        if text.contains('\\') {
            return self.class(text).into_iter().collect();
        }
        if let Some(full) = self.functions.get(&text.to_ascii_lowercase()) {
            return vec![full.clone()];
        }
        let mut out = vec![text.to_string()];
        if !self.namespace.is_empty() {
            out.push(self.qualify(text));
        }
        out
    }
}

fn text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

fn is_name(node: Node) -> bool {
    matches!(node.kind(), "name" | "qualified_name" | "relative_name")
}

/// Whether a name node sits where only a class can: a type, `new`, `::`,
/// `extends`, `implements`, a trait `use`, an attribute or `instanceof`.
fn class_position(node: Node) -> bool {
    let Some(p) = node.parent() else {
        return false;
    };
    match p.kind() {
        "named_type"
        | "base_clause"
        | "class_interface_clause"
        | "use_declaration"
        | "attribute"
        | "object_creation_expression" => true,
        "class_constant_access_expression" => p.child(0).is_some_and(|c| c.id() == node.id()),
        "scoped_call_expression" | "scoped_property_access_expression" => p
            .child_by_field_name("scope")
            .is_some_and(|c| c.id() == node.id()),
        "binary_expression" => {
            p.child_by_field_name("operator")
                .is_some_and(|o| o.kind() == "instanceof")
                && p.child_by_field_name("right")
                    .is_some_and(|c| c.id() == node.id())
        }
        _ => false,
    }
}

/// `use A\B, C\D as E;` and `use A\{B, C as D};`, into the scope, and each
/// imported name as a reference of its own.
fn import(decl: Node, source: &[u8], scope: &mut Scope, out: &mut Vec<(String, bool)>) {
    let kind_of = |n: Node| {
        n.child_by_field_name("type")
            .map(|t| text(t, source).to_string())
    };
    let outer = kind_of(decl);
    let mut prefix = String::new();
    let mut clauses = Vec::new();
    let mut walker = decl.walk();
    for child in decl.children(&mut walker) {
        match child.kind() {
            "namespace_name" => prefix = text(child, source).trim_start_matches('\\').to_string(),
            "namespace_use_clause" => clauses.push(child),
            "namespace_use_group" => {
                let mut w = child.walk();
                clauses.extend(
                    child
                        .children(&mut w)
                        .filter(|c| c.kind() == "namespace_use_clause"),
                );
            }
            _ => {}
        }
    }
    for clause in clauses {
        let kind = kind_of(clause).or_else(|| outer.clone());
        if kind.as_deref() == Some("const") {
            continue;
        }
        let mut w = clause.walk();
        let Some(name) = clause.named_children(&mut w).find(|c| is_name(*c)) else {
            continue;
        };
        let name = text(name, source).trim_start_matches('\\');
        let full = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}\\{name}")
        };
        let alias = clause
            .child_by_field_name("alias")
            .map(|a| text(a, source))
            .unwrap_or_else(|| full.rsplit('\\').next().unwrap_or(&full))
            .to_ascii_lowercase();
        let function = kind.as_deref() == Some("function");
        if function {
            scope.functions.insert(alias, full.clone());
        } else {
            scope.classes.insert(alias, full.clone());
        }
        out.push((full, function));
    }
}

/// The path an `include` or `require` names, when it's a literal, relative
/// to the file (`__DIR__ . '/x.php'`, `dirname(__DIR__) . '/x.php'`) or as
/// written (`'x.php'`).
fn included(expr: Node, source: &[u8]) -> Option<String> {
    let expr = if expr.kind() == "parenthesized_expression" {
        expr.named_child(0)?
    } else {
        expr
    };
    match expr.kind() {
        "string" | "encapsed_string" => literal(expr, source),
        "binary_expression" => {
            let right = literal(expr.child_by_field_name("right")?, source)?;
            let rest = right.strip_prefix('/')?;
            let ups = dir_depth(expr.child_by_field_name("left")?, source)?;
            Some(format!("./{}{rest}", "../".repeat(ups)))
        }
        _ => None,
    }
}

/// `__DIR__` is the file's own directory, each `dirname()` one level up.
fn dir_depth(node: Node, source: &[u8]) -> Option<usize> {
    if node.kind() == "name" && text(node, source) == "__DIR__" {
        return Some(0);
    }
    if node.kind() != "function_call_expression"
        || text(node.child_by_field_name("function")?, source) != "dirname"
    {
        return None;
    }
    let args = node.child_by_field_name("arguments")?;
    let mut w = args.walk();
    let list: Vec<Node> = args.named_children(&mut w).collect();
    let inner = dir_depth(list.first()?.named_child(0)?, source)?;
    let levels = match list.get(1) {
        Some(arg) => text(arg.named_child(0)?, source).parse().ok()?,
        None => 1,
    };
    Some(inner + levels)
}

/// A string's content when it has no interpolation.
fn literal(node: Node, source: &[u8]) -> Option<String> {
    if !matches!(node.kind(), "string" | "encapsed_string") {
        return None;
    }
    let mut w = node.walk();
    let mut out = String::new();
    for child in node.named_children(&mut w) {
        match child.kind() {
            "string_content" => out.push_str(text(child, source)),
            "escape_sequence" => {}
            _ => return None,
        }
    }
    Some(out)
}

fn parser() -> Option<Parser> {
    static LANGUAGE: OnceLock<Language> = OnceLock::new();
    let mut parser = Parser::new();
    parser.set_language(LANGUAGE.get_or_init(language)).ok()?;
    Some(parser)
}

/// A PHP file's declarations and references. Class references are specs
/// of kind `Import`, called functions of kind `Require`, both fully
/// qualified; included paths and path-like strings are literals.
pub fn extract(source: &[u8]) -> Extracted {
    let mut out = Extracted::default();
    let Some(tree) = parser().and_then(|mut p| p.parse(source, None)) else {
        return out;
    };
    let mut scope = Scope::default();
    let mut refs: Vec<(String, bool)> = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "namespace_definition" => {
                scope = Scope {
                    namespace: node
                        .child_by_field_name("name")
                        .map(|n| text(n, source).to_string())
                        .unwrap_or_default(),
                    ..Scope::default()
                };
            }
            "namespace_use_declaration" => {
                import(node, source, &mut scope, &mut refs);
                continue;
            }
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration"
            | "function_definition" => {
                if let Some(name) = node.child_by_field_name("name") {
                    out.declares.push(scope.qualify(text(name, source)));
                }
            }
            "function_call_expression" => {
                if let Some(f) = node.child_by_field_name("function").filter(|f| is_name(*f)) {
                    refs.extend(
                        scope
                            .function(text(f, source))
                            .into_iter()
                            .map(|n| (n, true)),
                    );
                }
            }
            "include_expression"
            | "include_once_expression"
            | "require_expression"
            | "require_once_expression" => {
                if let Some(path) = node.named_child(0).and_then(|e| included(e, source)) {
                    out.literals.push(path);
                    continue;
                }
            }
            "string" | "encapsed_string" => {
                if let Some(s) = literal(node, source).filter(|s| looks_like_path(s)) {
                    out.literals.push(s);
                }
            }
            _ if is_name(node) && class_position(node) => {
                if let Some(full) = scope.class(text(node, source)) {
                    refs.push((full, false));
                }
                continue;
            }
            _ => {}
        }
        let mut w = node.walk();
        let children: Vec<Node> = node.children(&mut w).collect();
        stack.extend(children.into_iter().rev());
    }
    out.specs = refs
        .into_iter()
        .map(|(name, function)| {
            let kind = if function {
                SpecKind::Require
            } else {
                SpecKind::Import
            };
            (name, kind)
        })
        .collect();
    out.specs.sort();
    out.specs.dedup();
    out.literals.sort();
    out.literals.dedup();
    out.declares.sort();
    out.declares.dedup();
    out
}

/// Composer's `psr-4` and `psr-0` prefixes, from every `composer.json` in
/// the tree, so path repositories count, and every class and function the
/// PHP files declare, for classmaps and anything autoloading doesn't cover.
#[derive(Debug, Default)]
pub struct Autoload {
    /// Namespace prefix with its trailing `\`, the directory, and whether
    /// it's PSR-0, which keeps the prefix in the path.
    prefixes: Vec<(String, String, bool)>,
    declared: HashMap<String, Vec<String>>,
    /// Each PHP declaration as written, with its file, for search.
    listed: Vec<(String, String)>,
}

/// Where a name resolved.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolved {
    Files(Vec<String>),
    /// Under one of the repository's own prefixes, but no file declares it:
    /// these are the paths autoloading would look for.
    Missing(Vec<String>),
    /// A vendor or built-in name.
    External,
}

impl Autoload {
    pub fn new<'a>(
        tree: &Tree,
        declared: impl Iterator<Item = (&'a str, &'a [String])>,
    ) -> Autoload {
        let mut prefixes = Vec::new();
        for manifest in tree
            .files
            .iter()
            .filter(|f| *f == "composer.json" || f.ends_with("/composer.json"))
            .filter(|f| !f.starts_with("vendor/") && !f.contains("/vendor/"))
        {
            let Ok(text) = std::fs::read_to_string(tree.abs(manifest)) else {
                continue;
            };
            prefixes.extend(autoload_prefixes(parent(manifest), &text));
        }
        // Longest prefix first, as composer tries them.
        prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.cmp(b)));
        let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
        let mut listed = Vec::new();
        for (file, names) in declared {
            for name in names {
                if file.ends_with(".php") {
                    listed.push((name.clone(), file.to_string()));
                }
                by_name
                    .entry(name.to_ascii_lowercase())
                    .or_default()
                    .push(file.to_string());
            }
        }
        listed.sort();
        Autoload {
            prefixes,
            declared: by_name,
            listed,
        }
    }

    /// Every class, interface, trait, enum and function a PHP file
    /// declares, as (qualified name, file), sorted.
    pub fn declarations(&self) -> &[(String, String)] {
        &self.listed
    }

    pub fn resolve(&self, tree: &Tree, name: &str) -> Resolved {
        let mut expected = Vec::new();
        for (prefix, dir, psr0) in &self.prefixes {
            let Some(rest) = name.strip_prefix(prefix.as_str()) else {
                continue;
            };
            let relative = if *psr0 { name } else { rest };
            let path = format!("{}.php", relative.replace('\\', "/"));
            if let Some(path) = normalize(dir, &path) {
                if tree.contains(&path) {
                    return Resolved::Files(vec![path]);
                }
                expected.push(path);
            }
        }
        if let Some(files) = self.declared.get(&name.to_ascii_lowercase()) {
            return Resolved::Files(files.clone());
        }
        if expected.is_empty() {
            Resolved::External
        } else {
            Resolved::Missing(expected)
        }
    }
}

/// `autoload` and `autoload-dev` prefixes of one manifest, with directories
/// made repo-relative.
fn autoload_prefixes(dir: &str, text: &str) -> Vec<(String, String, bool)> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for section in ["autoload", "autoload-dev"] {
        for (standard, psr0) in [("psr-4", false), ("psr-0", true)] {
            let Some(map) = json[section][standard].as_object() else {
                continue;
            };
            for (prefix, dirs) in map {
                let dirs: Vec<&str> = match dirs {
                    serde_json::Value::String(s) => vec![s.as_str()],
                    serde_json::Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
                    _ => continue,
                };
                for d in dirs {
                    let d = d.trim_start_matches("./").trim_end_matches('/');
                    let path = match d {
                        "" | "." => Some(dir.to_string()),
                        _ => normalize(dir, d),
                    };
                    if let Some(path) = path {
                        out.push((prefix.clone(), path, psr0));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(src: &str) -> Vec<(String, SpecKind)> {
        extract(src.as_bytes()).specs
    }

    fn classes(src: &str) -> Vec<String> {
        refs(src)
            .into_iter()
            .filter(|(_, k)| *k == SpecKind::Import)
            .map(|(n, _)| n)
            .collect()
    }

    #[test]
    fn class_references_resolve_through_imports_and_the_namespace() {
        let src = r#"<?php
namespace App\Http\Controllers;

use App\Models\User;
use App\Services\{Billing, Mail as Mailer};
use Illuminate\Support;

#[Attr\Route]
class UserController extends Controller implements \Contracts\HasName
{
    use Concerns\Loggable;

    public function show(User $user, ?Billing $b): Support\Collection
    {
        $m = new Mailer();
        if ($user instanceof Admin) {}
        return Support\Collection::make(Status::ACTIVE, static::class, self::X);
    }
}
"#;
        assert_eq!(
            classes(src),
            [
                "App\\Http\\Controllers\\Admin",
                "App\\Http\\Controllers\\Attr\\Route",
                "App\\Http\\Controllers\\Concerns\\Loggable",
                "App\\Http\\Controllers\\Controller",
                "App\\Http\\Controllers\\Status",
                "App\\Models\\User",
                "App\\Services\\Billing",
                "App\\Services\\Mail",
                "Contracts\\HasName",
                "Illuminate\\Support",
                "Illuminate\\Support\\Collection",
            ]
        );
        assert_eq!(
            extract(src.as_bytes()).declares,
            ["App\\Http\\Controllers\\UserController"]
        );
    }

    #[test]
    fn functions_includes_and_a_file_without_a_namespace() {
        let src = r#"<?php
use function App\Support\helper;
require __DIR__ . '/../vendor/autoload.php';
require_once dirname(__DIR__, 2) . '/bootstrap/app.php';
include 'config/app.php';
include $dynamic . '.php';
helper();
\App\other();
local_thing();
function declared_here() {}
$view = 'resources/views/home.blade.php';
$greeting = "hello $name/x.php";
"#;
        let e = extract(src.as_bytes());
        let functions: Vec<&str> = e
            .specs
            .iter()
            .filter(|(_, k)| *k == SpecKind::Require)
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(
            functions,
            ["App\\Support\\helper", "App\\other", "local_thing"]
        );
        assert_eq!(e.declares, ["declared_here"]);
        assert_eq!(
            e.literals,
            [
                "./../../bootstrap/app.php",
                "./../vendor/autoload.php",
                "config/app.php",
                "resources/views/home.blade.php",
            ]
        );
    }

    #[test]
    fn braced_namespaces_each_start_with_no_imports() {
        let src = r#"<?php
namespace A { use X\Y; new Y; }
namespace B { new Y; }
"#;
        assert_eq!(classes(src), ["B\\Y", "X\\Y"]);
    }

    #[test]
    fn psr4_then_declarations_then_missing_under_an_own_prefix() {
        let root = std::env::temp_dir().join(format!("fairlead-php-{}", std::process::id()));
        std::fs::create_dir_all(root.join("packages/billing")).unwrap();
        std::fs::write(
            root.join("composer.json"),
            r#"{"autoload":{"psr-4":{"App\\":"app/","Legacy\\":["lib/", "old/"]}},
               "autoload-dev":{"psr-4":{"Tests\\":"tests/"}}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("packages/billing/composer.json"),
            r#"{"autoload":{"psr-4":{"Billing\\":"src"}}}"#,
        )
        .unwrap();
        let files: Vec<String> = [
            "composer.json",
            "app/Models/User.php",
            "old/Thing.php",
            "packages/billing/composer.json",
            "packages/billing/src/Invoice.php",
            "database/classmap/Seeder.php",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let tree = Tree::from_files(&root, files);
        let seeder = vec!["DatabaseSeeder".to_string()];
        let auto = Autoload::new(
            &tree,
            [("database/classmap/Seeder.php", seeder.as_slice())].into_iter(),
        );
        let files = |v: &[&str]| Resolved::Files(v.iter().map(|s| s.to_string()).collect());
        assert_eq!(
            auto.resolve(&tree, "App\\Models\\User"),
            files(&["app/Models/User.php"])
        );
        assert_eq!(
            auto.resolve(&tree, "Legacy\\Thing"),
            files(&["old/Thing.php"])
        );
        assert_eq!(
            auto.resolve(&tree, "Billing\\Invoice"),
            files(&["packages/billing/src/Invoice.php"])
        );
        assert_eq!(
            auto.resolve(&tree, "databaseseeder"),
            files(&["database/classmap/Seeder.php"])
        );
        assert_eq!(
            auto.resolve(&tree, "App\\Models\\Gone"),
            Resolved::Missing(vec!["app/Models/Gone.php".into()])
        );
        assert_eq!(
            auto.resolve(&tree, "Illuminate\\Support\\Str"),
            Resolved::External
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
