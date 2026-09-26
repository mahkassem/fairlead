//! `[guard.comments]`: rules for what a comment says and how much space it
//! takes. Blocks come from the lines; a comment after code on the same line
//! comes from the syntax tree, so `//` in a string or JSX text never counts.

mod checks;
mod lines;

use fairlead_core::config::CommentRules;
use fairlead_core::pattern::Pattern;
use fairlead_lang::tree_sitter::Tree;
use regex::Regex;

use crate::finding::{Finding, Measure};
use crate::rules::{compile, Preset, Scope, Source};
use checks::{History, ItemCodes, Limits};
use lines::{Block, Kind, Style};

pub(crate) struct Comments {
    scope: Scope,
    tests: Vec<Pattern>,
    migrations: Vec<Pattern>,
    limits: Option<Limits>,
    density: Option<(f64, Option<f64>)>,
    history: Option<History>,
    item_codes: Option<ItemCodes>,
    agent_phrases: Vec<Regex>,
    block_marker: bool,
    ratchet: bool,
}

const RULES: [&str; 6] = [
    "block-length",
    "density",
    "history",
    "item-code",
    "agent-instruction",
    "block-marker",
];

impl Comments {
    pub(crate) fn new(c: &CommentRules) -> Result<Comments, String> {
        Ok(Comments {
            scope: Scope::new(c.files.items(), c.exclude.items())?,
            tests: compile(c.tests.items())?,
            migrations: compile(c.migrations.items())?,
            limits: c.block_length.as_ref().map(|b| Limits {
                source: b.source,
                test: b.test,
                header: b.header,
                inline: b.inline,
                migration: b.migration,
            }),
            density: c.density.as_ref().map(|d| (d.source, d.test)),
            history: c.history.as_ref().map(History::new),
            item_codes: c.item_codes.as_ref().map(ItemCodes::new).transpose()?,
            agent_phrases: c
                .agent_phrases
                .items()
                .iter()
                .map(|p| Regex::new(p).map_err(|e| e.to_string()))
                .collect::<Result<_, _>>()?,
            block_marker: c.block_marker,
            ratchet: c.ratchet,
        })
    }

    /// History, item codes and agent phrases: what a comment says, whether
    /// it is a block or follows code.
    fn says(&self, path: &str, b: &Block, out: &mut Vec<Finding>) {
        let found = |rule, message: String| Finding {
            file: path.to_string(),
            line: b.start,
            rule,
            message,
            anchor: b.flat.clone(),
            measure: None,
        };
        if let Some(hit) = self.history.as_ref().and_then(|h| h.check(&b.flat)) {
            out.push(found("history", format!("comment carries {hit}, which belongs in the change that made it, not in the code")));
        }
        if let Some(codes) = &self.item_codes {
            for code in codes.check(&b.flat) {
                let form = if codes.pointer.is_some() {
                    " outside the pointer form (CODE)."
                } else {
                    ""
                };
                out.push(found(
                    "item-code",
                    format!("comment names \"{code}\"{form}"),
                ));
            }
        }
        if let Some(phrase) = checks::agent_phrase(&self.agent_phrases, &b.flat) {
            out.push(found("agent-instruction", format!("comment addresses its next editor (\"{phrase}\") instead of describing the code")));
        }
    }

    fn block_length(&self, path: &str, b: &Block, test: bool, migration: bool) -> Option<Finding> {
        let (limit, context) = checks::block_limit(b, self.limits.as_ref()?, test, migration);
        (b.length > limit).then(|| Finding {
            file: path.to_string(),
            line: b.start,
            rule: "block-length",
            message: format!(
                "comment block is {} lines, max {limit} for {context}",
                b.length
            ),
            anchor: b.flat.clone(),
            measure: Some(Measure {
                size: u64::from(b.length),
                limit: u64::from(limit),
            }),
        })
    }

    fn density(&self, path: &str, kinds: &[Kind], test: bool) -> Option<Finding> {
        let (source, test_share) = self.density?;
        let max = if test {
            test_share.unwrap_or(source)
        } else {
            source
        };
        let share = checks::share(kinds);
        let permille = |v: f64| (v * 1000.0).round() as u64;
        (share > max).then(|| Finding {
            file: path.to_string(),
            line: 1,
            rule: "density",
            message: format!(
                "comments are {:.1}% of the file, over {}%",
                share * 100.0,
                permille(max) as f64 / 10.0
            ),
            anchor: String::new(),
            measure: Some(Measure {
                size: permille(share),
                limit: permille(max),
            }),
        })
    }
}

/// Every comment in a file as (line, flattened text): each block, and each
/// comment after code when the file has a syntax tree.
pub(crate) fn texts(source: &Source<'_>) -> Vec<(u32, String)> {
    let Some(style) = lines::style_of(source.path) else {
        return Vec::new();
    };
    let lines = lines::lines(source.text);
    let kinds = lines::classify(&lines, style);
    let mut out: Vec<(u32, String)> = lines::blocks(&lines, &kinds, style)
        .into_iter()
        .map(|b| (b.start, b.flat))
        .collect();
    if let Some(tree) = source.tree().filter(|_| style == Style::C) {
        out.extend(
            trailing(tree, source.text)
                .into_iter()
                .map(|b| (b.start, b.flat)),
        );
    }
    out
}

/// Each comment with code before it on its line, as a one-line block.
fn trailing(tree: &Tree, text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "comment" {
            let start = node.start_byte();
            let line_start = start - node.start_position().column;
            if !text[line_start..start].trim().is_empty() {
                out.push(Block {
                    start: node.start_position().row as u32 + 1,
                    length: 1,
                    flat: lines::flatten(&text[start..node.end_byte()], Style::C),
                    indented: false,
                    after_code: true,
                });
            }
            continue;
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    out
}

impl Preset for Comments {
    fn ratcheted(&self) -> Vec<&'static str> {
        if self.ratchet {
            RULES.to_vec()
        } else {
            Vec::new()
        }
    }

    fn applies(&self, path: &str) -> bool {
        self.scope.contains(path) && lines::style_of(path).is_some()
    }

    fn check(&self, source: &Source<'_>) -> Vec<Finding> {
        let Some(style) = lines::style_of(source.path) else {
            return Vec::new();
        };
        let path = source.path;
        let test = self.tests.iter().any(|p| p.is_match(path));
        let migration = self.migrations.iter().any(|p| p.is_match(path));
        let lines = lines::lines(source.text);
        let kinds = lines::classify(&lines, style);
        let mut out = Vec::new();
        for b in lines::blocks(&lines, &kinds, style) {
            out.extend(self.block_length(path, &b, test, migration));
            self.says(path, &b, &mut out);
        }
        if style == Style::C {
            if self.block_marker {
                out.extend(
                    checks::unmarked_lines(&lines)
                        .into_iter()
                        .map(|line| Finding {
                            file: path.to_string(),
                            line,
                            rule: "block-marker",
                            message:
                                "line inside a multi-line comment block doesn't start with \"*\""
                                    .into(),
                            anchor: lines[line as usize - 1].trim().to_string(),
                            measure: None,
                        }),
                );
            }
            let needs_tree = self.history.is_some()
                || self.item_codes.is_some()
                || !self.agent_phrases.is_empty();
            if let Some(tree) = source.tree().filter(|_| needs_tree) {
                for b in trailing(tree, source.text) {
                    self.says(path, &b, &mut out);
                }
            }
        }
        if !migration {
            out.extend(self.density(path, &kinds, test));
        }
        out
    }
}
