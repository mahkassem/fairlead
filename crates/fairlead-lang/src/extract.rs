//! What a source file refers to: module specifiers (imports, re-exports,
//! dynamic imports, `require`, test-runner mocks) and path-like string
//! literals. One compiled query per grammar, shared across threads.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

/// Files larger than this are almost always generated; they get a lexical
/// scan instead of a parse.
pub const LEXICAL_ABOVE_BYTES: usize = 256 * 1024;
const MAX_LITERAL_LEN: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SpecKind {
    Import,
    TypeImport,
    Dynamic,
    Require,
    Mock,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Extracted {
    pub specs: Vec<(String, SpecKind)>,
    /// String literals that look like file paths.
    pub literals: Vec<String>,
    /// A dynamic import whose argument isn't a plain string.
    pub unknown_dynamic: bool,
}

const COMMON: &str = r#"
(import_statement source: (string (string_fragment) @import))
(export_statement source: (string (string_fragment) @import))
(call_expression function: (import) arguments: (arguments . (string (string_fragment) @dynamic)))
(call_expression function: (import) arguments: (arguments . [(identifier) (template_string) (binary_expression) (member_expression) (call_expression) (subscript_expression)] @unknown))
(call_expression function: (identifier) @_req arguments: (arguments . (string (string_fragment) @require)) (#eq? @_req "require"))
(call_expression function: (identifier) @_dreq arguments: (arguments . [(identifier) (template_string) (binary_expression) (member_expression) (call_expression) (subscript_expression)] @unknown) (#eq? @_dreq "require"))
(call_expression function: (member_expression object: (identifier) @_obj property: (property_identifier) @_prop) arguments: (arguments . (string (string_fragment) @mock)) (#match? @_obj "^(vi|jest)$") (#match? @_prop "^(mock|doMock|unmock|requireActual|importActual|importMock|requireMock)$"))
(call_expression function: (member_expression object: (identifier) @_robj property: (property_identifier) @_rprop) arguments: (arguments . (string (string_fragment) @require)) (#eq? @_robj "require") (#eq? @_rprop "resolve"))
(string (string_fragment) @literal)
"#;
const TYPESCRIPT_ONLY: &str = r#"
(import_require_clause source: (string (string_fragment) @require))
"#;

#[derive(Clone, Copy)]
enum Grammar {
    TypeScript,
    Tsx,
    JavaScript,
}

fn grammar(rel: &str) -> Option<Grammar> {
    match rel.rsplit_once('.')?.1 {
        "ts" | "mts" | "cts" => Some(Grammar::TypeScript),
        "tsx" => Some(Grammar::Tsx),
        "js" | "jsx" | "mjs" | "cjs" => Some(Grammar::JavaScript),
        _ => None,
    }
}

fn language(g: Grammar) -> Language {
    match g {
        Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
    }
}

fn compiled(g: Grammar) -> &'static (Language, Query) {
    static QUERIES: OnceLock<[(Language, Query); 3]> = OnceLock::new();
    let all = QUERIES.get_or_init(|| {
        let build = |language: Language, typescript: bool| {
            let source = if typescript {
                format!("{COMMON}{TYPESCRIPT_ONLY}")
            } else {
                COMMON.to_string()
            };
            let query = Query::new(&language, &source).expect("the extraction query compiles");
            (language, query)
        };
        [
            build(language(Grammar::TypeScript), true),
            build(language(Grammar::Tsx), true),
            build(language(Grammar::JavaScript), false),
        ]
    });
    &all[g as usize]
}

/// What besides a file's bytes and extension decides what `extract`
/// returns: the queries, the size limits and each grammar's shape. The
/// parse cache is keyed on it, so none of these can serve a stale entry.
pub fn fingerprint() -> String {
    let mut hasher = sha1_smol::Sha1::new();
    hasher.update(COMMON.as_bytes());
    hasher.update(TYPESCRIPT_ONLY.as_bytes());
    hasher.update(format!("{LEXICAL_ABOVE_BYTES}/{MAX_LITERAL_LEN}").as_bytes());
    for g in [Grammar::TypeScript, Grammar::Tsx, Grammar::JavaScript] {
        let language = language(g);
        let shape = format!(
            "/{}/{}/{}",
            language.abi_version(),
            language.node_kind_count(),
            language.field_count()
        );
        hasher.update(shape.as_bytes());
    }
    hasher.digest().to_string()
}

/// A syntax tree for a JavaScript or TypeScript file, chosen by extension,
/// or none for any other file or one tree-sitter can't take. Byte offsets
/// and lines are the source's, whatever `unwrap_tag_types` blanked.
pub fn parse(rel: &str, source: &[u8]) -> Option<tree_sitter::Tree> {
    let g = grammar(rel)?;
    let mut parser = Parser::new();
    parser.set_language(&language(g)).ok()?;
    match g {
        Grammar::JavaScript => parser.parse(source, None),
        Grammar::TypeScript | Grammar::Tsx => parser.parse(unwrap_tag_types(source), None),
    }
}

/// `sql<Row>` before a template, with the `<Row>` blanked to spaces: the
/// TypeScript grammar can't parse type arguments on a template tag, and
/// recovers by cutting the enclosing function short. Newlines stay, so
/// every position is where it was.
/// Type arguments longer than this aren't looked for.
const TAG_TYPES_MAX_BYTES: usize = 2000;

fn unwrap_tag_types(source: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let mut out: Option<Vec<u8>> = None;
    for (tick, _) in source.iter().enumerate().filter(|(_, &b)| b == b'`') {
        let Some(close) = source[..tick]
            .iter()
            .rposition(|b| !b.is_ascii_whitespace())
        else {
            continue;
        };
        if source[close] != b'>' || close == 0 || source[close - 1] == b'=' {
            continue;
        }
        let mut depth = 0usize;
        let mut open = None;
        for i in (close.saturating_sub(TAG_TYPES_MAX_BYTES)..=close).rev() {
            match source[i] {
                // `=>` in a function type isn't a closing bracket.
                b'>' if i == 0 || source[i - 1] != b'=' => depth += 1,
                b'<' => {
                    depth -= 1;
                    if depth == 0 {
                        open = Some(i);
                        break;
                    }
                }
                b'`' => break,
                _ => {}
            }
        }
        let Some(open) = open else { continue };
        let tag_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
        if open == 0 || !tag_char(source[open - 1]) {
            continue;
        }
        let buf = out.get_or_insert_with(|| source.to_vec());
        for b in &mut buf[open..=close] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    }
    match out {
        Some(buf) => std::borrow::Cow::Owned(buf),
        None => std::borrow::Cow::Borrowed(source),
    }
}

pub fn extract(rel: &str, source: &[u8]) -> Extracted {
    let Some(g) = grammar(rel) else {
        return Extracted::default();
    };
    if source.len() > LEXICAL_ABOVE_BYTES {
        return lexical(source);
    }
    let (language, query) = compiled(g);
    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return lexical(source);
    }
    let Some(tree) = parser.parse(source, None) else {
        return lexical(source);
    };
    let names = query.capture_names();
    let mut out = Extracted::default();
    let mut module_nodes = std::collections::HashSet::new();
    let mut literals: Vec<(usize, String)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        for capture in m.captures() {
            let name = names[capture.index as usize];
            let node = capture.node;
            let Ok(text) = node.utf8_text(source) else {
                continue;
            };
            let kind = match name {
                "import" if is_type_only(node, source) => SpecKind::TypeImport,
                "import" => SpecKind::Import,
                "dynamic" => SpecKind::Dynamic,
                "require" => SpecKind::Require,
                "mock" => SpecKind::Mock,
                "unknown" => {
                    out.unknown_dynamic = true;
                    continue;
                }
                "literal" => {
                    literals.push((node.id(), text.to_string()));
                    continue;
                }
                _ => continue,
            };
            module_nodes.insert(node.id());
            out.specs.push((text.to_string(), kind));
        }
    }
    out.literals = literals
        .into_iter()
        .filter(|(id, text)| !module_nodes.contains(id) && looks_like_path(text))
        .map(|(_, text)| text)
        .collect();
    out.specs.sort();
    out.specs.dedup();
    out.literals.sort();
    out.literals.dedup();
    out
}

/// `import type { A } from "x"` and `export type { A } from "x"`.
fn is_type_only(fragment: tree_sitter::Node, source: &[u8]) -> bool {
    let statement = fragment.parent().and_then(|string| string.parent());
    statement
        .and_then(|s| s.utf8_text(source).ok())
        .is_some_and(|text| {
            let rest = text
                .trim_start()
                .trim_start_matches("import")
                .trim_start_matches("export")
                .trim_start();
            rest.starts_with("type ") || rest.starts_with("type{")
        })
}

/// A literal that could name a file: a path separator or a leading dot, a
/// file extension, no spaces, and not a URL or a scoped package.
pub fn looks_like_path(text: &str) -> bool {
    if text.len() > MAX_LITERAL_LEN
        || text.contains(char::is_whitespace)
        || text.contains("://")
        || text.starts_with('@')
    {
        return false;
    }
    let has_separator = text.contains('/') || text.starts_with('.');
    let last = text.rsplit('/').next().unwrap_or(text);
    let has_extension = last.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && !ext.is_empty()
            && ext.len() <= 10
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    });
    has_separator && has_extension
}

/// Import, export, `import()` and `require()` strings found without parsing.
fn lexical(source: &[u8]) -> Extracted {
    let text = String::from_utf8_lossy(source);
    let mut specs = Vec::new();
    for key in ["from ", "import(", "require(", "import "] {
        let mut rest: &str = &text;
        while let Some(at) = rest.find(key) {
            rest = &rest[at + key.len()..];
            let trimmed = rest.trim_start();
            let Some(quote) = trimmed.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
                continue;
            };
            if let Some(end) = trimmed[1..].find(quote) {
                let spec = &trimmed[1..1 + end];
                if !spec.is_empty() && !spec.contains('\n') {
                    specs.push((spec.to_string(), SpecKind::Import));
                }
            }
        }
    }
    specs.sort();
    specs.dedup();
    Extracted {
        specs,
        literals: Vec::new(),
        unknown_dynamic: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(rel: &str, src: &str) -> Vec<(String, SpecKind)> {
        extract(rel, src.as_bytes()).specs
    }

    #[test]
    fn finds_every_kind_of_specifier() {
        let src = r#"
import a from "./a.js";
import type { B } from "./b";
export { c } from "../c";
export * from "./d";
const e = await import("./e");
const f = require("./f");
vi.mock("./g");
jest.requireActual("./h");
require.resolve("./i");
import j = require("./j");
"#;
        let found = specs("src/x.ts", src);
        let expect = [
            ("../c", SpecKind::Import),
            ("./a.js", SpecKind::Import),
            ("./b", SpecKind::TypeImport),
            ("./d", SpecKind::Import),
            ("./e", SpecKind::Dynamic),
            ("./f", SpecKind::Require),
            ("./g", SpecKind::Mock),
            ("./h", SpecKind::Mock),
            ("./i", SpecKind::Require),
            ("./j", SpecKind::Require),
        ];
        for (spec, kind) in expect {
            assert!(
                found.contains(&(spec.to_string(), kind)),
                "missing {spec} {kind:?} in {found:?}"
            );
        }
    }

    #[test]
    fn a_non_literal_dynamic_import_is_unknown() {
        assert!(extract("x.js", b"const m = await import(name);").unknown_dynamic);
        assert!(extract("x.ts", b"await import(`./p/${n}.js`);").unknown_dynamic);
        assert!(!extract("x.ts", b"await import('./p.js');").unknown_dynamic);
        assert!(extract("x.cjs", b"const m = require(path.join(dir, name));").unknown_dynamic);
        assert!(!extract("x.cjs", b"const m = require('./m');").unknown_dynamic);
    }

    #[test]
    fn path_like_literals_are_kept_and_module_specifiers_are_not() {
        let src = r#"import a from "./a.json"; const bin = "../../cli/bin/run.mjs"; const s = "hello world"; const u = "https://x.io/a.js";"#;
        let got = extract("t/x.test.ts", src.as_bytes());
        assert_eq!(got.literals, vec!["../../cli/bin/run.mjs".to_string()]);
    }

    #[test]
    fn a_template_tag_with_type_arguments_parses_as_a_tagged_template() {
        let src = "const f = (a: string) =>\n  sql<{ n: Array<number>; f: (a: string) => void }>`\n    select ${a}\n  `\nconst g = (x: number) => x > 1 ? `a` : `b`\n";
        let tree = parse("a.ts", src.as_bytes()).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
        let arrow = tree
            .root_node()
            .child(0)
            .unwrap()
            .named_child(0)
            .unwrap()
            .child_by_field_name("value")
            .unwrap();
        assert_eq!(
            (arrow.kind(), arrow.end_position().row),
            ("arrow_function", 3)
        );
        assert_eq!(&src[arrow.start_byte()..arrow.start_byte() + 3], "(a:");
    }

    #[test]
    fn large_files_are_scanned_lexically() {
        let mut src = String::from("import { a } from \"./a\";\nconst x = require('./b');\n");
        src.push_str(&"// filler\n".repeat(LEXICAL_ABOVE_BYTES / 10 + 1));
        let got = specs("big.ts", &src);
        assert_eq!(
            got,
            vec![
                ("./a".to_string(), SpecKind::Import),
                ("./b".to_string(), SpecKind::Import)
            ]
        );
    }
}
