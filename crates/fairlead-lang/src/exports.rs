//! What a JavaScript or TypeScript module says about names: the ones it
//! declares and exports, the ones it re-exports from elsewhere, and the
//! ones it imports by name. The scan uses them to follow a named import
//! through a barrel to the file that declares the name.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tree_sitter::Node;

/// A module's names, as far as its syntax shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// Names the module declares itself and exports; `default` for a
    /// default export.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reexports: Vec<ReExport>,
    /// Specifiers every import of which, value or type, names its bindings
    /// in braces, with the names; any other form keeps the whole module.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<NamedImport>,
    /// Exports more than `local` and `reexports` show (`export =`,
    /// CommonJS, or no `export` at all), so a name not found is unknown
    /// rather than missing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub opaque: bool,
}

/// `export * from "s"` (no names), `export { a as b } from "s"` (names as
/// exported and as imported), or `export * as ns from "s"` (imported `*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReExport {
    pub spec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<Vec<(String, String)>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub type_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedImport {
    pub spec: String,
    pub names: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub type_only: bool,
}

/// The names imported in braces, or `None` for any other form: a default,
/// a namespace, `import x = require()` or a bare `import "s"`.
fn braced_names(statement: Node, source: &[u8]) -> Option<Vec<String>> {
    let mut cursor = statement.walk();
    let clause = statement
        .named_children(&mut cursor)
        .find(|c| c.kind() == "import_clause")?;
    let mut cursor = clause.walk();
    let parts: Vec<Node> = clause.named_children(&mut cursor).collect();
    let [named] = parts.as_slice() else {
        return None;
    };
    if named.kind() != "named_imports" {
        return None;
    }
    let mut cursor = named.walk();
    let names = named
        .named_children(&mut cursor)
        .filter(|s| s.kind() == "import_specifier")
        .map(|s| {
            s.child_by_field_name("name")
                .and_then(|n| name_text(n, source))
        })
        .collect::<Option<Vec<String>>>()?;
    // `import {} from "s"` names nothing, so it's there to load the module.
    (!names.is_empty()).then_some(names)
}

/// An identifier's text, or a string name's contents (`import { "a-b" as c }`).
fn name_text(node: Node, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    Some(match node.kind() {
        "string" => text.trim_matches(|c| c == '"' || c == '\'').to_string(),
        _ => text.to_string(),
    })
}

fn has_token(node: Node, token: &str) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|c| !c.is_named() && c.kind() == token);
    found
}

fn source_spec(statement: Node, source: &[u8]) -> Option<String> {
    let string = statement.child_by_field_name("source")?;
    let mut cursor = string.walk();
    let fragment = string
        .named_children(&mut cursor)
        .find(|c| c.kind() == "string_fragment");
    match fragment {
        Some(f) => f.utf8_text(source).ok().map(str::to_string),
        None => Some(String::new()),
    }
}

/// `export … from "s"`: what it re-exports.
fn reexport(statement: Node, source: &[u8], spec: String, type_only: bool) -> ReExport {
    let mut cursor = statement.walk();
    let children: Vec<Node> = statement.named_children(&mut cursor).collect();
    let names = children.iter().find_map(|c| match c.kind() {
        "namespace_export" => {
            let mut w = c.walk();
            let name = c
                .named_children(&mut w)
                .find_map(|n| name_text(n, source))
                .unwrap_or_default();
            Some(vec![(name, "*".to_string())])
        }
        "export_clause" => {
            let mut w = c.walk();
            let pairs = c
                .named_children(&mut w)
                .filter(|s| s.kind() == "export_specifier")
                .filter_map(|s| {
                    let name = name_text(s.child_by_field_name("name")?, source)?;
                    let alias = s
                        .child_by_field_name("alias")
                        .and_then(|a| name_text(a, source))
                        .unwrap_or_else(|| name.clone());
                    Some((alias, name))
                })
                .collect();
            Some(pairs)
        }
        _ => None,
    });
    ReExport {
        spec,
        names,
        type_only,
    }
}

/// The names a declaration binds: its `name`, or each variable's, with
/// destructuring patterns taken apart.
fn declared(node: Node, source: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for d in node.named_children(&mut cursor) {
                if let Some(name) = d.child_by_field_name("name") {
                    bound(name, source, out);
                }
            }
        }
        "ambient_declaration" => {
            let mut cursor = node.walk();
            for d in node.named_children(&mut cursor) {
                declared(d, source, out);
            }
        }
        _ => {
            if let Some(name) = node.child_by_field_name("name") {
                out.extend(name_text(name, source));
            }
        }
    }
}

/// Every identifier a binding pattern binds.
fn bound(pattern: Node, source: &[u8], out: &mut Vec<String>) {
    match pattern.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            out.extend(name_text(pattern, source));
        }
        // `{ key: value }` binds `value`; a default's right side binds nothing.
        "pair_pattern" => {
            if let Some(value) = pattern.child_by_field_name("value") {
                bound(value, source, out);
            }
        }
        "assignment_pattern" | "object_assignment_pattern" => {
            if let Some(left) = pattern.child_by_field_name("left") {
                bound(left, source, out);
            }
        }
        _ => {
            let mut cursor = pattern.walk();
            for c in pattern.named_children(&mut cursor) {
                bound(c, source, out);
            }
        }
    }
}

/// An `export` with no `from`: the names it makes public, or `None` for
/// `export = x`, which replaces the module's exports wholesale.
fn local_export(statement: Node, source: &[u8], out: &mut Vec<String>) -> Option<()> {
    if has_token(statement, "default") {
        out.push("default".into());
        return Some(());
    }
    if has_token(statement, "=") {
        return None;
    }
    if let Some(declaration) = statement.child_by_field_name("declaration") {
        declared(declaration, source, out);
        return Some(());
    }
    let mut cursor = statement.walk();
    for clause in statement
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "export_clause")
    {
        let mut w = clause.walk();
        for s in clause.named_children(&mut w) {
            let exported = s
                .child_by_field_name("alias")
                .or_else(|| s.child_by_field_name("name"));
            out.extend(exported.and_then(|n| name_text(n, source)));
        }
    }
    Some(())
}

/// Reads a module's names: each import or export statement with a source
/// as the extraction query finds it, wherever it is (an ambient `declare
/// module` block holds them too), then the top level's own exports.
#[derive(Default)]
pub struct Reader {
    out: Shape,
    /// Each specifier and whether the import is type-only, with its names,
    /// or `None` once one import of it took more than names.
    groups: BTreeMap<(String, bool), Option<Vec<String>>>,
}

impl Reader {
    /// An `import_statement`, or an `export_statement` with a source, and
    /// whether it's type-only. A re-export in a `declare module` block is
    /// that block's, not the file's.
    pub fn statement(&mut self, statement: Node, source: &[u8], type_only: bool) {
        let Some(spec) = source_spec(statement, source) else {
            return;
        };
        if statement.kind() == "export_statement" {
            if statement.parent().is_some_and(|p| p.kind() == "program") {
                self.out
                    .reexports
                    .push(reexport(statement, source, spec, type_only));
            }
            return;
        }
        if statement.kind() != "import_statement" {
            return;
        }
        let key = (spec, type_only);
        let names = braced_names(statement, source);
        let group = self.groups.entry(key).or_insert_with(|| Some(Vec::new()));
        match (group.as_mut(), names) {
            (Some(all), Some(names)) => all.extend(names),
            _ => *group = None,
        }
    }

    pub fn finish(mut self, root: Node, source: &[u8]) -> Shape {
        let mut exports_any = !self.out.reexports.is_empty();
        let mut cursor = root.walk();
        for node in root.named_children(&mut cursor) {
            if node.kind() != "export_statement" || node.child_by_field_name("source").is_some() {
                continue;
            }
            exports_any = true;
            if local_export(node, source, &mut self.out.local).is_none() {
                self.out.opaque = true;
            }
        }
        self.out.opaque |= !exports_any;
        self.out.imports = std::mem::take(&mut self.groups)
            .into_iter()
            .filter_map(|((spec, type_only), names)| {
                let mut names = names?;
                names.sort();
                names.dedup();
                Some(NamedImport {
                    spec,
                    names,
                    type_only,
                })
            })
            .collect();
        self.out.local.sort();
        self.out.local.dedup();
        self.out.reexports.sort_by(|a, b| a.spec.cmp(&b.spec));
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape_of(rel: &str, src: &str) -> Shape {
        crate::extract::extract(rel, src.as_bytes())
            .shape
            .expect("a parsed module has a shape")
    }

    fn import(spec: &str, names: &[&str], type_only: bool) -> NamedImport {
        NamedImport {
            spec: spec.into(),
            names: names.iter().map(|n| n.to_string()).collect(),
            type_only,
        }
    }

    #[test]
    fn only_specifiers_every_import_of_which_names_its_bindings_are_named() {
        let src = r#"
import { a, b as c } from "./lib";
import { d } from "./lib";
import type { T } from "./lib";
import { type U, e } from "./types";
import { f } from "./ns";
import * as ns from "./ns";
import D from "./def";
import G, { g } from "./mixed";
import "./side";
import {} from "./empty";
import x = require("./req");
"#;
        let got = shape_of("src/x.ts", src).imports;
        assert_eq!(
            got,
            vec![
                import("./lib", &["a", "b", "d"], false),
                import("./lib", &["T"], true),
                import("./types", &["U", "e"], false),
            ]
        );
    }

    #[test]
    fn re_exports_keep_their_names_as_exported_and_as_imported() {
        let src = r#"
export * from "./all";
export { a, b as c, default as D } from "./some";
export * as ns from "./space";
export type { T } from "./types";
declare module "elsewhere" { export * from "./ambient"; }
"#;
        let got = shape_of("src/index.ts", src).reexports;
        let pair = |e: &str, o: &str| (e.to_string(), o.to_string());
        assert_eq!(
            got,
            vec![
                ReExport {
                    spec: "./all".into(),
                    names: None,
                    type_only: false
                },
                ReExport {
                    spec: "./some".into(),
                    names: Some(vec![pair("a", "a"), pair("c", "b"), pair("D", "default")]),
                    type_only: false
                },
                ReExport {
                    spec: "./space".into(),
                    names: Some(vec![pair("ns", "*")]),
                    type_only: false
                },
                ReExport {
                    spec: "./types".into(),
                    names: Some(vec![pair("T", "T")]),
                    type_only: true
                },
            ]
        );
    }

    #[test]
    fn local_exports_are_every_name_a_declaration_or_clause_makes_public() {
        let src = r#"
export const a = 1, { b, c: d, e = 2, ...rest } = o, [f, , g] = arr;
export let h;
export function i() {}
export async function* j() {}
export class K {}
export abstract class L {}
export interface M {}
export type N = string;
export enum O {}
export const enum P {}
export declare const q: number;
export declare function r(): void;
export namespace S {}
const t = 1, u = 2;
export { t, u as v };
export default function () {}
function hidden() {}
"#;
        let shape = shape_of("src/x.ts", src);
        let want = [
            "K", "L", "M", "N", "O", "P", "S", "a", "b", "d", "default", "e", "f", "g", "h", "i",
            "j", "q", "r", "rest", "t", "v",
        ];
        assert_eq!(shape.local, want);
        assert!(!shape.opaque);
    }

    #[test]
    fn a_module_whose_exports_its_syntax_doesnt_show_is_opaque() {
        assert!(shape_of("x.ts", "const a = 1;\nexport = a;\n").opaque);
        assert!(shape_of("x.cjs", "module.exports = { a: 1 };\n").opaque);
        assert!(shape_of("x.ts", "const a = 1;\n").opaque);
        assert!(!shape_of("x.ts", "export * from './a';\n").opaque);
        assert!(!shape_of("x.ts", "export {};\n").opaque);
    }

    #[test]
    fn a_component_exports_its_default_and_keeps_its_named_imports() {
        let vue = "<script lang=\"ts\">\nimport { a } from './lib';\nimport { b } from './both';\n</script>\n<script setup>\nimport * as all from './both'\n</script>";
        let shape = shape_of("x.vue", vue);
        assert_eq!(shape.local, ["default"]);
        assert!(shape.opaque);
        assert_eq!(shape.imports, vec![import("./lib", &["a"], false)]);
    }

    #[test]
    fn a_lexically_scanned_file_has_no_shape() {
        let mut src = String::from("export * from './a';\n");
        src.push_str(&"// filler\n".repeat(crate::extract::LEXICAL_ABOVE_BYTES / 10 + 1));
        assert!(crate::extract::extract("big.ts", src.as_bytes())
            .shape
            .is_none());
    }
}
