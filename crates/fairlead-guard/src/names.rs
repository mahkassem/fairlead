//! `[guard.test_names]`: a test file's name, and the titles of its tests.

use std::collections::HashSet;

use fairlead_core::config::TestNames;
use fairlead_lang::tree_sitter::Node;
use regex::Regex;

use crate::finding::Finding;
use crate::functions::callee_name;
use crate::rules::{Preset, Scope, Source};

pub(crate) struct Names {
    scope: Scope,
    file: Option<Regex>,
    titles_without: Option<Regex>,
    calls: HashSet<String>,
}

impl Names {
    pub(crate) fn new(c: &TestNames) -> Result<Names, String> {
        let compile = |p: &Option<String>| {
            p.as_deref()
                .map(|p| Regex::new(p).map_err(|e| e.to_string()))
                .transpose()
        };
        Ok(Names {
            scope: Scope::new(c.files.items(), c.exclude.items())?,
            file: compile(&c.file)?,
            titles_without: compile(&c.titles_without)?,
            calls: c.title_calls.items().iter().cloned().collect(),
        })
    }
}

/// The text of a string or template literal with no substitutions.
fn literal<'a>(node: Node<'_>, text: &'a str) -> Option<&'a str> {
    let raw = node.utf8_text(text.as_bytes()).ok()?;
    match node.kind() {
        "string" => raw.get(1..raw.len().checked_sub(1)?),
        "template_string" if !raw.contains("${") => raw.get(1..raw.len().checked_sub(1)?),
        _ => None,
    }
}

/// Each title call's line and title.
fn titles<'a>(root: Node<'_>, text: &'a str, calls: &HashSet<String>) -> Vec<(u32, &'a str)> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression" {
            let named = node
                .child_by_field_name("function")
                .and_then(|f| callee_name(f, text))
                .is_some_and(|name| calls.contains(name));
            let first = node
                .child_by_field_name("arguments")
                .and_then(|args| args.named_child(0));
            if let (true, Some(arg)) = (named, first) {
                if let Some(title) = literal(arg, text) {
                    out.push((arg.start_position().row as u32 + 1, title));
                }
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    out.sort_by_key(|(line, _)| *line);
    out
}

impl Preset for Names {
    fn ratcheted(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn applies(&self, path: &str) -> bool {
        self.scope.contains(path)
    }

    fn check(&self, source: &Source<'_>) -> Vec<Finding> {
        let found = |line, rule, message: String, anchor: &str| Finding {
            file: source.path.to_string(),
            line,
            rule,
            message,
            anchor: anchor.to_string(),
            measure: None,
        };
        let mut out = Vec::new();
        let name = source.path.rsplit('/').next().unwrap_or(source.path);
        if self.file.as_ref().is_some_and(|re| !re.is_match(name)) {
            out.push(found(
                1,
                "test-file-name",
                format!("file name `{name}` doesn't match the test file pattern"),
                "",
            ));
        }
        let (Some(without), Some(tree)) = (&self.titles_without, source.tree()) else {
            return out;
        };
        for (line, title) in titles(tree.root_node(), source.text, &self.calls) {
            if let Some(m) = without.find(title) {
                out.push(found(
                    line,
                    "test-title",
                    format!("test title carries \"{}\"", m.as_str()),
                    title,
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(file: Option<&str>, without: Option<&str>) -> Names {
        Names::new(&TestNames {
            files: vec!["**".to_string()].into(),
            file: file.map(String::from),
            titles_without: without.map(String::from),
            ..TestNames::default()
        })
        .unwrap()
    }

    fn check(n: &Names, path: &str, text: &str) -> Vec<String> {
        n.check(&Source::new(path, text))
            .iter()
            .map(|f| f.to_string())
            .collect()
    }

    #[test]
    fn a_file_name_outside_the_pattern_is_a_finding() {
        let n = names(Some(r"^[a-z]+(-[a-z]+)*\.test\.ts$"), None);
        assert!(check(&n, "test/leave-balance.test.ts", "").is_empty());
        assert_eq!(
            check(&n, "test/LeaveBalance.test.ts", ""),
            ["test/LeaveBalance.test.ts:1 test-file-name: file name `LeaveBalance.test.ts` doesn't match the test file pattern"]
        );
    }

    #[test]
    fn a_title_that_carries_the_pattern_is_a_finding_wherever_the_call_sits() {
        let n = names(None, Some(r"(?-u:\b)T[0-9]{4}(?-u:\b)"));
        let text = "describe(\"T1024 leave\", () => {\n  it.only('works', () => {})\n  test(`T2000 edge`, () => {})\n  it(`${x} T3000`, () => {})\n})\n";
        assert_eq!(
            check(&n, "a.test.ts", text),
            [
                "a.test.ts:1 test-title: test title carries \"T1024\"",
                "a.test.ts:3 test-title: test title carries \"T2000\""
            ]
        );
    }
}
