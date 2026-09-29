//! `[guard.citations]`: a pointer in a comment, such as `(ABC-12)`, has to
//! name a heading in a Markdown file, so a pointer never points nowhere.

use std::collections::HashSet;
use std::path::Path;

use fairlead_core::config;
use regex::Regex;

use crate::finding::Finding;
use crate::rules::{Preset, Scope, Source};

pub(crate) struct Citations {
    scope: Scope,
    pattern: Regex,
    headings_in: String,
    headings: HashSet<String>,
}

/// Each heading's first word, with a trailing `:` or `.` dropped, so
/// `## ABC-12: title` names `ABC-12`.
pub(crate) fn heading_names(markdown: &str) -> HashSet<String> {
    markdown
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix('#'))
        .map(|rest| rest.trim_start_matches('#'))
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|word| word.trim_end_matches([':', '.']).to_string())
        .collect()
}

impl Citations {
    pub(crate) fn new(c: &config::Citations, root: &Path) -> Result<Citations, String> {
        let path = root.join(&c.headings_in);
        let markdown = std::fs::read_to_string(&path)
            .map_err(|e| format!("guard.citations.headings_in: {}: {e}", c.headings_in))?;
        Ok(Citations {
            scope: Scope::new(c.files.items(), c.exclude.items())?,
            pattern: Regex::new(&c.pattern).map_err(|e| e.to_string())?,
            headings_in: c.headings_in.clone(),
            headings: heading_names(&markdown),
        })
    }
}

impl Preset for Citations {
    fn table(&self) -> &'static str {
        "citations"
    }

    fn ratcheted(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn applies(&self, path: &str) -> bool {
        self.scope.contains(path)
    }

    fn check(&self, source: &Source<'_>) -> Vec<Finding> {
        let mut out = Vec::new();
        for (line, flat) in crate::comments::texts(source) {
            for caps in self.pattern.captures_iter(&flat) {
                let Some(code) = caps.name("code").map(|m| m.as_str()) else {
                    continue;
                };
                if !self.headings.contains(code) {
                    out.push(Finding {
                        file: source.path.to_string(),
                        line,
                        rule: "citation",
                        message: format!(
                            "cites \"{code}\", which no heading in {} names",
                            self.headings_in
                        ),
                        anchor: flat.clone(),
                        measure: None,
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_names_its_first_word_at_any_level() {
        let names =
            heading_names("# Decisions\n\n## ABC-12\ntext\n### no-cache: why\n#not a heading\n");
        assert!(
            names.contains("ABC-12") && names.contains("no-cache") && names.contains("Decisions")
        );
        assert!(!names.contains("not"));
    }

    #[test]
    fn a_pointer_to_a_missing_heading_is_a_finding() {
        let dir = std::env::temp_dir().join(format!("fairlead-citations-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("decisions.md"), "## ABC-12\n").unwrap();
        let c = Citations::new(
            &config::Citations {
                files: vec!["**".to_string()].into(),
                exclude: Default::default(),
                pattern: r"\((?P<code>[A-Z]+-[0-9]+)\)".into(),
                headings_in: "decisions.md".into(),
            },
            &dir,
        )
        .unwrap();
        let text = "// Why (ABC-12).\nx() // and why (ABC-99).\n";
        let found: Vec<String> = c
            .check(&Source::new("a.ts", text))
            .iter()
            .map(|f| f.to_string())
            .collect();
        assert_eq!(
            found,
            ["a.ts:2 citation: cites \"ABC-99\", which no heading in decisions.md names"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
