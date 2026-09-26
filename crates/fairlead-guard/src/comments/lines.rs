//! Lines as code, blank or comment, and runs of comment lines as blocks. A
//! line is a comment only when it starts with a comment marker once
//! trimmed, so a comment after code stays a code line here; those are found
//! from the syntax tree instead.

use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    /// `//` and `/* */`.
    C,
    /// `--`.
    Sql,
    /// `#`.
    Hash,
}

pub(crate) fn style_of(path: &str) -> Option<Style> {
    match path.rsplit_once('.')?.1 {
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Some(Style::C),
        "sql" => Some(Style::Sql),
        "yml" | "yaml" | "toml" | "sh" => Some(Style::Hash),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Code,
    Blank,
    Comment,
}

/// The file's lines; a final newline doesn't start one.
pub(crate) fn lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

pub(crate) fn classify(lines: &[&str], style: Style) -> Vec<Kind> {
    let mut kinds = Vec::with_capacity(lines.len());
    let mut in_block = false;
    for line in lines {
        let t = line.trim();
        let kind = match style {
            Style::C if in_block => {
                in_block = !t.contains("*/");
                Kind::Comment
            }
            _ if t.is_empty() => Kind::Blank,
            Style::C if t.starts_with("//") => Kind::Comment,
            Style::C if t.starts_with("/*") => {
                in_block = !t[2..].contains("*/");
                Kind::Comment
            }
            Style::C => Kind::Code,
            Style::Sql if t.starts_with("--") => Kind::Comment,
            Style::Hash if t.starts_with('#') => Kind::Comment,
            _ => Kind::Code,
        };
        kinds.push(kind);
    }
    kinds
}

/// A run of comment lines, or a single comment after code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    /// 1-based.
    pub start: u32,
    pub length: u32,
    /// Each line's marker stripped and whitespace collapsed, so a phrase
    /// split across a wrap reads as one.
    pub flat: String,
    /// The first line starts with whitespace.
    pub indented: bool,
    /// The nearest non-blank line above is code.
    pub after_code: bool,
}

fn marker(style: Style) -> &'static Regex {
    static MARKERS: OnceLock<[Regex; 3]> = OnceLock::new();
    let [c, sql, hash] = MARKERS.get_or_init(|| {
        [
            Regex::new(r"^\s*(?:/\*\*|/\*|\*/|//|\*)\s?").expect("valid"),
            Regex::new(r"^\s*--\s?").expect("valid"),
            Regex::new(r"^\s*#\s?").expect("valid"),
        ]
    });
    match style {
        Style::C => c,
        Style::Sql => sql,
        Style::Hash => hash,
    }
}

pub(crate) fn flatten(text: &str, style: Style) -> String {
    let stripped: Vec<String> = text
        .split('\n')
        .map(|line| marker(style).replace(line, "").into_owned())
        .collect();
    stripped
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn blocks(lines: &[&str], kinds: &[Kind], style: Style) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < kinds.len() {
        if kinds[i] != Kind::Comment {
            i += 1;
            continue;
        }
        let after_code = kinds[..i]
            .iter()
            .rev()
            .find(|k| **k != Kind::Blank)
            .is_some_and(|k| *k == Kind::Code);
        let first = i;
        while i < kinds.len() && kinds[i] == Kind::Comment {
            i += 1;
        }
        let text = lines[first..i].join("\n");
        let head = lines[first];
        blocks.push(Block {
            start: first as u32 + 1,
            length: (i - first) as u32,
            flat: flatten(&text, style),
            indented: head.starts_with(char::is_whitespace) && !head.trim().is_empty(),
            after_code,
        });
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_comment_runs_to_its_close_and_a_comment_after_code_is_code() {
        let text = "/* a\n b */\nx() // c\n  // d\n\n-- e\n";
        let kinds = classify(&lines(text), Style::C);
        use Kind::*;
        assert_eq!(kinds, [Comment, Comment, Code, Comment, Blank, Code]);
    }

    #[test]
    fn sql_and_hash_comments_use_their_own_marker() {
        assert_eq!(
            classify(&["-- a", "select 1"], Style::Sql),
            [Kind::Comment, Kind::Code]
        );
        assert_eq!(
            classify(&["# a", "a: 1"], Style::Hash),
            [Kind::Comment, Kind::Code]
        );
    }

    #[test]
    fn flattening_strips_each_lines_marker_and_joins_across_the_wrap() {
        assert_eq!(
            flatten("/**\n * used\n * to be */", Style::C),
            "used to be */"
        );
        assert_eq!(flatten("// a\n//   b", Style::C), "a b");
        assert_eq!(flatten("-- a\n--b", Style::Sql), "a b");
    }

    #[test]
    fn a_block_knows_its_start_length_indent_and_whether_code_is_above() {
        let text = "x()\n\n  // a\n  // b\ny()\n// c\n";
        let found = blocks(&lines(text), &classify(&lines(text), Style::C), Style::C);
        assert_eq!(found.len(), 2);
        assert_eq!(
            (
                found[0].start,
                found[0].length,
                found[0].indented,
                found[0].after_code
            ),
            (3, 2, true, true)
        );
        assert_eq!(found[0].flat, "a b");
        assert_eq!((found[1].start, found[1].indented), (6, false));
    }
}
