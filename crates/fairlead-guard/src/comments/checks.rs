//! What a comment block may not carry, and how long it may be. Each check
//! reads one block and returns what it found, if anything.

use regex::Regex;

use super::lines::{Block, Kind};

/// Block limits by context; a context without one falls through.
pub(crate) struct Limits {
    pub source: u32,
    pub test: Option<u32>,
    pub header: Option<u32>,
    pub inline: Option<u32>,
    pub migration: Option<u32>,
}

/// The limit and the name of the context it came from.
pub(crate) fn block_limit(
    b: &Block,
    l: &Limits,
    test: bool,
    migration: bool,
) -> (u32, &'static str) {
    let inline = b.after_code && b.indented;
    [
        (migration, l.migration, "migration"),
        (inline, l.inline, "inline"),
        (b.start == 1, l.header, "file header"),
        (test, l.test, "test"),
    ]
    .into_iter()
    .find_map(|(applies, limit, name)| applies.then_some(limit).flatten().map(|l| (l, name)))
    .unwrap_or((l.source, "source"))
}

pub(crate) struct History {
    pub date: Option<Regex>,
    /// Each name with its whole-word pattern.
    pub names: Vec<(String, Regex)>,
    /// Each lowercase phrase with its whole-word pattern.
    pub phrases: Vec<(String, Regex)>,
    pub measured: Option<(Regex, Regex)>,
}

fn whole_word(text: &str) -> Regex {
    Regex::new(&format!(r"(?-u:\b){}(?-u:\b)", regex::escape(text)))
        .expect("escaped text is a valid regex")
}

impl History {
    pub(crate) fn new(c: &fairlead_core::config::History) -> History {
        History {
            date: c
                .dates
                .then(|| Regex::new("20[0-9]{2}-[0-9]{2}-[0-9]{2}").expect("valid")),
            names: c.names.iter().map(|n| (n.clone(), whole_word(n))).collect(),
            phrases: c
                .phrases
                .iter()
                .map(|p| (p.to_lowercase(), whole_word(&p.to_lowercase())))
                .collect(),
            measured: c.measured.then(|| {
                (
                    Regex::new(r"[0-9]+ ?(?:px|ms|s|KB|MB)(?-u:\b)").expect("valid"),
                    Regex::new("(?i)measured").expect("valid"),
                )
            }),
        }
    }

    /// The first thing found, in order: a date, a name, a phrase, a measurement.
    pub(crate) fn check(&self, flat: &str) -> Option<String> {
        if self.date.as_ref().is_some_and(|re| re.is_match(flat)) {
            return Some("a date".into());
        }
        if let Some((name, _)) = self.names.iter().find(|(_, re)| re.is_match(flat)) {
            return Some(format!("\"{name}\""));
        }
        let lower = flat.to_lowercase();
        if let Some((phrase, _)) = self.phrases.iter().find(|(_, re)| re.is_match(&lower)) {
            return Some(format!("\"{phrase}\""));
        }
        let (number, word) = self.measured.as_ref()?;
        (number.is_match(flat) && word.is_match(flat))
            .then(|| "a measurement beside \"measured\"".into())
    }
}

pub(crate) struct ItemCodes {
    pub code: Regex,
    /// `code` anchored to the whole text, for the parts of a pointer.
    pub whole: Regex,
    pub pointer: Option<Regex>,
}

impl ItemCodes {
    pub(crate) fn new(c: &fairlead_core::config::ItemCodes) -> Result<ItemCodes, String> {
        let compile = |p: &str| Regex::new(p).map_err(|e| e.to_string());
        Ok(ItemCodes {
            code: compile(&c.pattern)?,
            whole: compile(&format!("^(?:{})$", c.pattern))?,
            pointer: c
                .pointer
                .then(|| compile(r"\(([^()]{1,120})\)(\.)?"))
                .transpose()?,
        })
    }

    /// Byte ranges of `(CODE)` or `(CODE, CODE)` followed by a full stop or
    /// the end of the text.
    fn pointers(&self, flat: &str) -> Vec<(usize, usize)> {
        let Some(re) = &self.pointer else {
            return Vec::new();
        };
        re.captures_iter(flat)
            .filter(|m| m[1].split(',').all(|part| self.whole.is_match(part.trim())))
            .filter_map(|m| {
                let span = m.get(0)?;
                let ends = m.get(2).is_some() || flat[span.end()..].trim().is_empty();
                ends.then_some((span.start(), span.end()))
            })
            .collect()
    }

    /// Each distinct code outside a pointer, once.
    pub(crate) fn check(&self, flat: &str) -> Vec<String> {
        let pointers = self.pointers(flat);
        let mut seen: Vec<String> = Vec::new();
        for m in self.code.find_iter(flat) {
            let inside = pointers
                .iter()
                .any(|&(s, e)| m.start() >= s && m.start() < e);
            if !inside && !seen.iter().any(|c| c == m.as_str()) {
                seen.push(m.as_str().to_string());
            }
        }
        seen
    }
}

/// The first agent phrase the comment matches, as written there.
pub(crate) fn agent_phrase(phrases: &[Regex], flat: &str) -> Option<String> {
    phrases
        .iter()
        .find_map(|re| re.find(flat))
        .map(|m| m.as_str().to_string())
}

/// 1-based lines inside a multi-line `/* */` that don't start with `*`.
pub(crate) fn unmarked_lines(lines: &[&str]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut in_block = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if !in_block {
            in_block = t.starts_with("/*") && !t[2..].contains("*/");
            continue;
        }
        if !t.starts_with('*') {
            out.push(i as u32 + 1);
        }
        in_block = !t.contains("*/");
    }
    out
}

/// Comment lines as a share of comment and code lines.
pub(crate) fn share(kinds: &[Kind]) -> f64 {
    let comment = kinds.iter().filter(|k| **k == Kind::Comment).count();
    let code = kinds.iter().filter(|k| **k == Kind::Code).count();
    if comment + code == 0 {
        0.0
    } else {
        comment as f64 / (comment + code) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fairlead_core::config;

    fn block(start: u32, indented: bool, after_code: bool) -> Block {
        Block {
            start,
            length: 3,
            flat: String::new(),
            indented,
            after_code,
        }
    }

    #[test]
    fn a_blocks_context_is_the_first_that_applies_with_a_limit() {
        let l = Limits {
            source: 8,
            test: Some(10),
            header: Some(12),
            inline: Some(2),
            migration: Some(1),
        };
        assert_eq!(
            block_limit(&block(5, true, true), &l, false, true),
            (1, "migration")
        );
        assert_eq!(
            block_limit(&block(5, true, true), &l, true, false),
            (2, "inline")
        );
        assert_eq!(
            block_limit(&block(1, false, false), &l, true, false),
            (12, "file header")
        );
        assert_eq!(
            block_limit(&block(5, true, false), &l, true, false),
            (10, "test")
        );
        assert_eq!(
            block_limit(&block(5, false, true), &l, false, false),
            (8, "source")
        );
        let bare = Limits {
            source: 8,
            test: None,
            header: None,
            inline: None,
            migration: None,
        };
        assert_eq!(
            block_limit(&block(1, true, true), &bare, true, true),
            (8, "source")
        );
    }

    fn history() -> History {
        History::new(&config::History {
            dates: true,
            names: vec!["Ada".into()],
            phrases: vec!["used to".into(), "on 2026".into()],
            measured: true,
        })
    }

    #[test]
    fn history_finds_dates_names_whole_word_phrases_and_measurements() {
        let h = history();
        assert_eq!(h.check("since 2026-01-02").as_deref(), Some("a date"));
        assert_eq!(h.check("ask Ada").as_deref(), Some("\"Ada\""));
        assert_eq!(h.check("ask Adam"), None);
        assert_eq!(h.check("it Used To wait").as_deref(), Some("\"used to\""));
        assert_eq!(h.check("it refused toast"), None);
        assert_eq!(
            h.check("measured at 40 ms").as_deref(),
            Some("a measurement beside \"measured\"")
        );
        assert_eq!(h.check("waits 40 ms"), None);
    }

    fn codes() -> ItemCodes {
        ItemCodes::new(&config::ItemCodes {
            pattern: r"(?-u:\b)T[0-9]{4}(?-u:\b)".into(),
            pointer: true,
        })
        .unwrap()
    }

    #[test]
    fn a_code_in_a_pointer_at_the_end_or_before_a_full_stop_is_allowed() {
        let c = codes();
        assert!(c.check("why this is so (T1024)").is_empty());
        assert!(c.check("why (T1024, T1025). More words").is_empty());
        assert_eq!(c.check("see (T1024) for more"), ["T1024"]);
        assert_eq!(c.check("T1024 and T1024 and T1025"), ["T1024", "T1025"]);
        assert_eq!(c.check("(T1024 and more)"), ["T1024"]);
    }

    #[test]
    fn a_multi_line_block_needs_a_star_on_each_continuation_line() {
        let lines = ["/**", " * ok", " not ok", " */", "/* one line */", "x()"];
        assert_eq!(unmarked_lines(&lines), [3]);
    }

    #[test]
    fn share_counts_comment_lines_among_comment_and_code() {
        use Kind::*;
        assert_eq!(share(&[Comment, Code, Blank, Code, Code]), 0.25);
        assert_eq!(share(&[Blank]), 0.0);
    }
}
