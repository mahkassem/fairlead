//! Java and Kotlin: a file refers to a class by its qualified name, through
//! an import, a wildcard import or its own package, which needs no import.
//! Every file's package and top-level declarations make an index of
//! qualified names, and a reference resolves through it once all are read.

use std::collections::HashMap;

use crate::extract::{Extracted, SpecKind};

/// The JVM scanner's id, in reports.
pub const ID: &str = "jvm";

pub fn is_jvm(file: &str) -> bool {
    file.ends_with(".java") || file.ends_with(".kt")
}

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Word(&'a str),
    Punct(char),
}

/// The file's words and punctuation, without comments, strings or char
/// literals.
fn tokens(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        let c = bytes[i];
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
        } else if let Some(body) = rest.strip_prefix("/*") {
            i += body.find("*/").map_or(rest.len(), |e| e + 4);
        } else if let Some(body) = rest.strip_prefix("\"\"\"") {
            i += body.find("\"\"\"").map_or(rest.len(), |e| e + 6);
        } else if c == b'"' || c == b'\'' {
            i += quoted(rest.as_bytes(), c);
        } else if c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80 {
            let len = rest
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
                .unwrap_or(rest.len());
            out.push(Token::Word(&rest[..len]));
            i += len.max(1);
        } else if c.is_ascii_digit() {
            let len = rest
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(rest.len());
            i += len.max(1);
        } else {
            if !c.is_ascii_whitespace() {
                out.push(Token::Punct(c as char));
            }
            i += 1;
        }
    }
    out
}

/// The length of the string or char literal at the start of `s`.
fn quoted(s: &[u8], quote: u8) -> usize {
    let mut i = 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    s.len()
}

/// A dotted name from `at`, and where it stops; `*` ends a wildcard.
fn dotted(tokens: &[Token<'_>], mut at: usize) -> (String, usize) {
    let mut name = String::new();
    while let Some(token) = tokens.get(at) {
        match token {
            Token::Word(w) if name.is_empty() || name.ends_with('.') => name.push_str(w),
            Token::Punct('.') if !name.is_empty() && !name.ends_with('.') => name.push('.'),
            Token::Punct('*') if name.ends_with('.') => name.push('*'),
            _ => break,
        }
        at += 1;
    }
    (name, at)
}

const DECLARES: [&str; 7] = [
    "class",
    "interface",
    "enum",
    "record",
    "object",
    "typealias",
    "fun",
];
/// Words that can stand before a declaration without being one.
const NOT_NAMES: [&str; 3] = ["data", "sealed", "companion"];

/// What the file says about itself and what it names.
#[derive(Debug, Default, PartialEq)]
struct Read {
    package: String,
    imports: Vec<String>,
    wildcards: Vec<String>,
    declares: Vec<String>,
    used: Vec<String>,
}

fn read(text: &str, kotlin: bool) -> Read {
    let tokens = tokens(text);
    let mut out = Read::default();
    let mut depth = 0usize;
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Punct('{') => depth += 1,
            Token::Punct('}') => depth = depth.saturating_sub(1),
            Token::Word("package") if depth == 0 && out.package.is_empty() => {
                let (name, end) = dotted(&tokens, i + 1);
                out.package = name;
                i = end;
                continue;
            }
            Token::Word("import") if depth == 0 => {
                let from = i + 1 + usize::from(tokens.get(i + 1) == Some(&Token::Word("static")));
                let (name, end) = dotted(&tokens, from);
                match name.strip_suffix(".*") {
                    Some(prefix) => out.wildcards.push(prefix.to_string()),
                    None if !name.is_empty() => out.imports.push(name),
                    None => {}
                }
                // Kotlin's `import a.B as C` names B's file, as C, in this one.
                let alias = tokens.get(end) == Some(&Token::Word("as"));
                i = end + if alias { 2 } else { 0 };
                continue;
            }
            Token::Word(w) if depth == 0 && DECLARES.contains(w) => {
                match declared(&tokens, i + 1, *w == "fun") {
                    Some(name) if !out.declares.iter().any(|d| d == name) => {
                        out.declares.push(name.to_string())
                    }
                    _ => {}
                }
            }
            Token::Word(w) => {
                let call = kotlin && tokens.get(i + 1) == Some(&Token::Punct('('));
                if w.starts_with(|c: char| c.is_uppercase()) || call {
                    out.used.push(w.to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    out.used.sort();
    out.used.dedup();
    out
}

/// The name a declaration keyword introduces: the next word, or for a
/// Kotlin `fun` the last word before its parameters, past type parameters
/// and an extension's receiver.
fn declared<'a>(tokens: &[Token<'a>], mut at: usize, function: bool) -> Option<&'a str> {
    if !function {
        return match tokens.get(at)? {
            Token::Word(w) if !NOT_NAMES.contains(w) && !DECLARES.contains(w) => Some(w),
            Token::Word("class" | "interface") => declared(tokens, at + 1, false),
            _ => None,
        };
    }
    let mut angle = 0usize;
    let mut last = None;
    while let Some(token) = tokens.get(at) {
        match token {
            Token::Punct('<') => angle += 1,
            Token::Punct('>') => angle = angle.saturating_sub(1),
            Token::Punct('(') if angle == 0 => return last,
            Token::Word(w) if angle == 0 => last = Some(*w),
            Token::Punct('.' | '?') => {}
            _ if angle == 0 => return None,
            _ => {}
        }
        at += 1;
    }
    None
}

/// Explicit imports as `Import` specs, which may name a member below the
/// class; candidates from wildcards and the file's own package as `Require`
/// specs, which only an exact declaration matches. The file's own
/// qualified names are what it declares.
pub fn extract(rel: &str, source: &[u8]) -> Extracted {
    let text = String::from_utf8_lossy(source);
    let r = read(&text, rel.ends_with(".kt"));
    let qualify = |prefix: &str, name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        }
    };
    let mut specs: Vec<(String, SpecKind)> = r
        .imports
        .iter()
        .map(|i| (i.clone(), SpecKind::Import))
        .collect();
    for prefix in std::iter::once(&r.package).chain(&r.wildcards) {
        specs.extend(
            r.used
                .iter()
                .filter(|u| !r.declares.contains(u))
                .map(|u| (qualify(prefix, u), SpecKind::Require)),
        );
    }
    specs.sort();
    specs.dedup();
    let mut declares: Vec<String> = r.declares.iter().map(|d| qualify(&r.package, d)).collect();
    declares.sort();
    declares.dedup();
    Extracted {
        specs,
        declares,
        ..Extracted::default()
    }
}

/// Every qualified name the tree's Java and Kotlin files declare.
#[derive(Debug, Default)]
pub struct Index {
    declared: HashMap<String, Vec<String>>,
}

impl Index {
    pub fn new<'a>(files: impl Iterator<Item = (&'a str, &'a [String])>) -> Index {
        let mut declared: HashMap<String, Vec<String>> = HashMap::new();
        for (file, names) in files.filter(|(f, _)| is_jvm(f)) {
            for name in names {
                declared
                    .entry(name.clone())
                    .or_default()
                    .push(file.to_string());
            }
        }
        Index { declared }
    }

    /// Every top-level declaration, as (qualified name, file), sorted.
    pub fn declarations(&self) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = self
            .declared
            .iter()
            .flat_map(|(name, files)| files.iter().map(move |f| (name.as_str(), f.as_str())))
            .collect();
        out.sort_unstable();
        out
    }

    /// The files that declare a name. An import may name a nested class or
    /// a static member, so it's tried without its last parts too; a
    /// candidate matches exactly or not at all. A name outside the tree,
    /// such as the JDK's, gives none.
    pub fn resolve(&self, name: &str, kind: SpecKind) -> Vec<String> {
        let mut name = name;
        loop {
            if let Some(files) = self.declared.get(name) {
                return files.clone();
            }
            match name.rsplit_once('.') {
                Some((head, _)) if kind == SpecKind::Import && head.contains('.') => name = head,
                _ => return Vec::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_java_file_names_its_package_imports_declarations_and_the_types_it_uses() {
        let text = r#"
            package com.acme.orders; // the package
            import java.util.List;
            import static com.acme.util.Money.format;
            import com.acme.model.*;
            /* class Hidden {} */
            public final class OrderService implements Service {
                private final String note = "class Fake {";
                List<Order> open() { return Repo.find('c'); }
                static class Inner {}
            }
            @interface Audited {}
        "#;
        let r = read(text, false);
        assert_eq!(r.package, "com.acme.orders");
        assert_eq!(r.imports, ["java.util.List", "com.acme.util.Money.format"]);
        assert_eq!(r.wildcards, ["com.acme.model"]);
        assert_eq!(r.declares, ["OrderService", "Audited"]);
        assert_eq!(
            r.used,
            [
                "Audited",
                "Inner",
                "List",
                "Order",
                "OrderService",
                "Repo",
                "Service",
                "String"
            ]
        );
    }

    #[test]
    fn a_kotlin_file_declares_top_level_functions_and_names_its_calls() {
        let text = "package app\n\nimport app.model.User as Person\n\ndata class Box<T>(val t: T)\nenum class Color { RED }\nsealed interface Shape\nobject Registry { fun inner() = 1 }\nfun <T> List<T>.second(): T = this[1]\nfun greet(p: Person) = format(\"hi ${p.name}\")\n";
        let r = read(text, true);
        assert_eq!(r.package, "app");
        assert_eq!(r.imports, ["app.model.User"]);
        assert_eq!(
            r.declares,
            ["Box", "Color", "Shape", "Registry", "second", "greet"]
        );
        assert!(r.used.contains(&"format".to_string()) && r.used.contains(&"Person".to_string()));
    }

    #[test]
    fn names_resolve_through_imports_wildcards_and_the_same_package() {
        let files = [
            ("src/main/java/com/acme/Order.java", "package com.acme;\npublic class Order { public static class Line {} }\n"),
            ("src/main/java/com/acme/model/User.java", "package com.acme.model;\npublic record User(String name) {}\n"),
            ("src/main/java/com/acme/util/Money.java", "package com.acme.util;\npublic class Money { static String format(long c) { return \"\"; } }\n"),
            ("src/test/java/com/acme/OrderTest.java", "package com.acme;\nimport static com.acme.util.Money.format;\nimport com.acme.model.*;\nclass OrderTest { Order.Line l; User u; }\n"),
        ];
        let extracted: Vec<(String, Extracted)> = files
            .iter()
            .map(|(f, t)| (f.to_string(), extract(f, t.as_bytes())))
            .collect();
        let index = Index::new(
            extracted
                .iter()
                .map(|(f, e)| (f.as_str(), e.declares.as_slice())),
        );
        let test = &extracted[3].1;
        let mut reached: Vec<String> = test
            .specs
            .iter()
            .flat_map(|(name, kind)| index.resolve(name, *kind))
            .collect();
        reached.sort();
        reached.dedup();
        assert_eq!(
            reached,
            [
                "src/main/java/com/acme/Order.java",
                "src/main/java/com/acme/model/User.java",
                "src/main/java/com/acme/util/Money.java",
            ]
        );
        assert!(index.resolve("java.util.List", SpecKind::Import).is_empty());
        assert!(index
            .resolve("com.acme.Missing", SpecKind::Require)
            .is_empty());
    }
}
