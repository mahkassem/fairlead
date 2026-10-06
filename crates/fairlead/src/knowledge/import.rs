//! A lessons document a team already keeps, read as one lesson per `## `
//! heading: the split, the title, the scope from backticked paths, the links
//! and a body that fits the cap. Nothing here touches the filesystem.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use fairlead_core::pattern::Pattern;
use regex::Regex;

/// One `## ` heading and the lines under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub heading: String,
    /// The heading's line in the document, from 1.
    pub line: usize,
    /// GitHub's anchor for the heading, with `-1`, `-2` on a repeat.
    pub anchor: String,
    /// The lines under it, without blank lines at either end.
    pub body: Vec<String>,
    /// The document line of `body[0]`.
    pub body_line: usize,
}

#[derive(Debug, Default)]
pub struct Split {
    pub sections: Vec<Section>,
    /// Non-blank lines before the first `## ` heading.
    pub preamble: usize,
    /// Non-blank lines under a later `# ` heading, which no `## ` claims.
    pub outside: usize,
}

fn fence(line: &str) -> Option<&str> {
    let t = line.trim_start_matches(' ');
    (line.len() - t.len() <= 3)
        .then(|| ["```", "~~~"].into_iter().find(|f| t.starts_with(f)))
        .flatten()
}

/// The heading level and text of an ATX heading line, if it is one.
fn heading(line: &str) -> Option<(usize, String)> {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return None;
    }
    let level = t.bytes().take_while(|&b| b == b'#').count();
    let rest = &t[level..];
    if level == 0 || level > 6 || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let text = rest.trim();
    let closed = text.trim_end_matches('#');
    let text = if closed.is_empty() || closed.ends_with([' ', '\t']) {
        closed.trim_end()
    } else {
        text
    };
    Some((level, text.to_string()))
}

/// GitHub's anchor: lowercase, spaces to dashes, other punctuation dropped.
pub fn anchor(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

fn close_section(s: Option<Section>, out: &mut Split) {
    if let Some(mut s) = s {
        let lead = s.body.iter().take_while(|l| l.trim().is_empty()).count();
        s.body.drain(..lead);
        s.body_line += lead;
        while s.body.last().is_some_and(|l| l.trim().is_empty()) {
            s.body.pop();
        }
        out.sections.push(s);
    }
}

/// Splits a document at its `## ` headings, outside fenced code.
pub fn split(text: &str) -> Split {
    let mut out = Split::default();
    let mut current: Option<Section> = None;
    let mut open_fence: Option<&str> = None;
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut before_first = true;
    for (i, line) in text.lines().enumerate() {
        let level = match open_fence {
            Some(f) => {
                if fence(line) == Some(f) {
                    open_fence = None;
                }
                None
            }
            None => {
                open_fence = fence(line);
                heading(line)
            }
        };
        match level {
            Some((2, h)) => {
                close_section(current.take(), &mut out);
                before_first = false;
                let base = anchor(&h);
                let n = seen.entry(base.clone()).or_insert(0);
                let anchor = if *n == 0 { base } else { format!("{base}-{n}") };
                *n += 1;
                current = Some(Section {
                    heading: h,
                    line: i + 1,
                    anchor,
                    body: Vec::new(),
                    body_line: i + 2,
                });
            }
            Some((1, _)) if !before_first => {
                close_section(current.take(), &mut out);
                out.outside += 1;
            }
            _ => match current.as_mut() {
                Some(s) => s.body.push(line.to_string()),
                None if line.trim().is_empty() => {}
                None if before_first => out.preamble += 1,
                None => out.outside += 1,
            },
        }
    }
    close_section(current, &mut out);
    out
}

/// A heading that is only a code (`AB12`, `QA-7`) or a slug (`keep-diffs-small`)
/// names the lesson rather than saying it.
pub fn names_only(heading: &str) -> bool {
    static CODE: OnceLock<Regex> = OnceLock::new();
    let code = CODE.get_or_init(|| {
        Regex::new(r"^\(?[A-Za-z]{1,5}[-_]?[0-9]{1,6}\)?$").expect("the code pattern compiles")
    });
    let h = heading.trim().trim_matches('`');
    !h.is_empty()
        && !h.contains(char::is_whitespace)
        && (code.is_match(h)
            || h.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'))
}

/// The longest title, in characters.
pub const TITLE_MAX: usize = 100;

/// The heading when it reads as a title; else the body's first sentence.
pub fn title(heading: &str, body: &[String]) -> String {
    if !names_only(heading) && !heading.trim().is_empty() {
        return heading.trim().to_string();
    }
    first_sentence(body).unwrap_or_else(|| heading.trim().to_string())
}

fn first_sentence(body: &[String]) -> Option<String> {
    let mut words: Vec<&str> = Vec::new();
    for line in body {
        let t = line.trim();
        if t.is_empty() || fence(line).is_some() || heading(line).is_some() {
            if words.is_empty() && t.is_empty() {
                continue;
            }
            break;
        }
        let t = t.trim_start_matches(['>', '-', '*', '+', ' ']);
        words.extend(t.split_whitespace());
    }
    let text = words.join(" ").replace("**", "").replace("__", "");
    let end = text
        .match_indices(['.', '!', '?'])
        .map(|(i, _)| i + 1)
        .find(|&i| text[i..].starts_with(' ') || i == text.len())
        .unwrap_or(text.len());
    let sentence = text[..end].trim();
    (!sentence.is_empty()).then(|| shorten(sentence, TITLE_MAX))
}

/// At most `max` characters, cut at a word where one is near and marked `…`.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max - 1).collect();
    let at_word = cut
        .rfind(char::is_whitespace)
        .filter(|&i| i > max / 2)
        .map_or(cut.as_str(), |i| &cut[..i]);
    format!("{}…", at_word.trim_end_matches([' ', ',', ';', ':']))
}

/// The repository's files and the directories that hold them.
pub struct Known {
    files: HashSet<String>,
    dirs: HashSet<String>,
    list: Vec<String>,
}

impl Known {
    pub fn new(list: Vec<String>) -> Known {
        let mut dirs = HashSet::new();
        for f in &list {
            let mut d = f.as_str();
            while let Some((parent, _)) = d.rsplit_once('/') {
                if !dirs.insert(parent.to_string()) {
                    break;
                }
                d = parent;
            }
        }
        Known {
            files: list.iter().cloned().collect(),
            dirs,
            list,
        }
    }

    /// The scope a backticked path gives: a file as it is, a directory as
    /// `dir/**`, a glob when it matches a file.
    fn scope(&self, raw: &str) -> Option<String> {
        static SUFFIX: OnceLock<Regex> = OnceLock::new();
        let suffix = SUFFIX.get_or_init(|| {
            Regex::new(r"(?::\d+(?:[:-]\d+)?|#L\d+(?:-L?\d+)?)$").expect("the suffix compiles")
        });
        let p = raw.trim();
        let p = suffix.replace(p, "");
        let p = p.strip_prefix("./").unwrap_or(&p).trim_end_matches('/');
        if p.is_empty()
            || p == "."
            || p.starts_with('/')
            || p.contains(char::is_whitespace)
            || p.split('/').any(|part| part == ".." || part.is_empty())
        {
            return None;
        }
        // A name like `app/[id]/page.tsx` is a file before it is a glob.
        let literal = || {
            p.chars()
                .flat_map(|c| {
                    let meta = matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\');
                    meta.then_some('\\').into_iter().chain([c])
                })
                .collect::<String>()
        };
        if self.files.contains(p) {
            Some(literal())
        } else if self.dirs.contains(p) {
            Some(format!("{}/**", literal()))
        } else if p.contains(['*', '?', '[', '{']) {
            let glob = Pattern::new(p).ok()?;
            self.list.iter().any(|f| glob.is_match(f)).then(|| p.into())
        } else {
            None
        }
    }
}

/// The text between single backticks on each line, outside fenced code.
fn backticked(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut open_fence: Option<&str> = None;
    for line in lines {
        if let Some(f) = open_fence {
            if fence(line) == Some(f) {
                open_fence = None;
            }
            continue;
        }
        if let Some(f) = fence(line) {
            open_fence = Some(f);
            continue;
        }
        let parts: Vec<&str> = line.split('`').collect();
        // Odd pieces sit between a pair; a last unpaired backtick opens nothing.
        let paired = if parts.len().is_multiple_of(2) {
            parts.len() - 1
        } else {
            parts.len()
        };
        out.extend(
            parts[..paired]
                .iter()
                .skip(1)
                .step_by(2)
                .map(|s| s.to_string()),
        );
    }
    out
}

/// The scope the heading and its body name, in the order they name it.
pub fn paths(heading: &str, body: &[String], known: &Known) -> Vec<String> {
    let mut lines: Vec<&str> = vec![heading];
    lines.extend(body.iter().map(String::as_str));
    let mut out: Vec<String> = Vec::new();
    for raw in backticked(&lines) {
        if let Some(p) = known.scope(&raw) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// Every http(s) link in the text, once each, in order.
pub fn links(body: &[String]) -> Vec<String> {
    static LINK: OnceLock<Regex> = OnceLock::new();
    let link = LINK.get_or_init(|| {
        Regex::new(r#"https?://[^\s<>()\[\]`'"]+"#).expect("the link pattern compiles")
    });
    let mut out: Vec<String> = Vec::new();
    for line in body {
        for m in link.find_iter(line) {
            let l = m
                .as_str()
                .trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_']);
            if !out.iter().any(|o| o == l) {
                out.push(l.to_string());
            }
        }
    }
    out
}

/// The body as a lesson keeps it: whole when it fits `max`, else the first
/// `max - 1` lines and a last line linking the full text.
pub fn capped(body: &[String], max: usize, full: &str) -> (String, bool) {
    if body.len() <= max {
        return (body.join("\n"), false);
    }
    let mut kept: Vec<&str> = body
        .iter()
        .take(max.saturating_sub(1))
        .map(String::as_str)
        .collect();
    // A cut inside fenced code would leave the link inside the fence.
    let mut opened: Option<(usize, &str)> = None;
    for (i, l) in kept.iter().enumerate() {
        match (opened, fence(l)) {
            (Some((_, f)), Some(g)) if f == g => opened = None,
            (None, Some(g)) => opened = Some((i, g)),
            _ => {}
        }
    }
    if let Some((i, _)) = opened {
        kept.truncate(i);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    let link = format!("Full text: {full}");
    kept.push(&link);
    (kept.join("\n"), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn a_document_splits_at_level_two_headings_and_skips_the_preamble() {
        let doc = "# Lessons\n\nIntro line.\n\n## First rule\n\nBody one.\n### A detail\nmore\n\n## AB12\n```\n## not a heading\n```\n## First rule\n\n# Appendix\nloose\n";
        let s = split(doc);
        assert_eq!(s.preamble, 2, "the title and the intro");
        assert_eq!(s.outside, 2, "the appendix heading and its line");
        let heads: Vec<(&str, usize, &str)> = s
            .sections
            .iter()
            .map(|x| (x.heading.as_str(), x.line, x.anchor.as_str()))
            .collect();
        assert_eq!(
            heads,
            [
                ("First rule", 5, "first-rule"),
                ("AB12", 11, "ab12"),
                ("First rule", 15, "first-rule-1"),
            ]
        );
        assert_eq!(s.sections[0].body, ["Body one.", "### A detail", "more"]);
        assert_eq!(s.sections[0].body_line, 7);
        assert_eq!(s.sections[1].body, ["```", "## not a heading", "```"]);
        assert!(s.sections[2].body.is_empty());
    }

    #[test]
    fn a_heading_that_is_a_code_or_a_slug_takes_the_first_sentence_as_title() {
        assert!(names_only("AB12") && names_only("QA-7") && names_only("(XY58)"));
        assert!(names_only("keep-diffs-small") && names_only("`no-global-state`"));
        assert!(!names_only("Fix the rule") && !names_only("Caching"));
        let body = lines("\n**Totals are computed once.** Never twice.\nMore.");
        assert_eq!(title("AB12", &body), "Totals are computed once.");
        assert_eq!(title("Compute late", &body), "Compute late");
        let long = lines(&"word ".repeat(60));
        let t = title("keep-diffs-small", &long);
        assert!(t.chars().count() <= TITLE_MAX && t.ends_with('…'), "{t}");
        assert_eq!(title("ab12", &[]), "ab12", "no body keeps the heading");
    }

    #[test]
    fn only_backticked_paths_that_exist_become_the_scope() {
        let known = Known::new(vec![
            "src/billing/total.ts".into(),
            "src/billing/invoice.ts".into(),
            "README.md".into(),
            "app/[id]/page.tsx".into(),
        ]);
        let body = lines(
            "See `src/billing/total.ts:12` and `./src/billing/`, not `src/gone.ts`.\n`README.md` twice: `README.md`, `../x`, `src/*.ts`, `src/**/*.ts`\n```\n`src/billing/invoice.ts`\n```",
        );
        assert_eq!(
            paths("Totals in `src/billing`", &body, &known),
            [
                "src/billing/**",
                "src/billing/total.ts",
                "README.md",
                "src/**/*.ts"
            ]
        );
        assert!(paths("Nothing", &lines("`npm test` and `main`"), &known).is_empty());
        let route = paths("Route", &lines("`app/[id]/page.tsx`"), &known);
        assert_eq!(route, [r"app/\[id\]/page.tsx"]);
        assert!(Pattern::new(&route[0])
            .unwrap()
            .is_match("app/[id]/page.tsx"));
    }

    #[test]
    fn links_are_collected_once_without_trailing_punctuation() {
        let body = lines("See https://example.com/pr/1. And [run](https://example.com/run/2), https://example.com/pr/1");
        assert_eq!(
            links(&body),
            ["https://example.com/pr/1", "https://example.com/run/2"]
        );
    }

    #[test]
    fn a_long_body_keeps_the_cap_and_links_the_full_text() {
        let body: Vec<String> = (1..=20).map(|i| format!("line {i}")).collect();
        let (text, cut) = capped(&body, 12, "docs/lessons.md#ab12");
        assert!(cut);
        let kept: Vec<&str> = text.lines().collect();
        assert_eq!(kept.len(), 12);
        assert_eq!(kept[10], "line 11");
        assert_eq!(kept[11], "Full text: docs/lessons.md#ab12");
        let short = lines("one\ntwo");
        assert_eq!(capped(&short, 12, "x"), ("one\ntwo".to_string(), false));
        let fenced = lines("a\n```\nb\nc\nd\n```\ne");
        let (text, _) = capped(&fenced, 4, "x#y");
        assert_eq!(text, "a\nFull text: x#y", "never cut inside a fence");
    }
}
