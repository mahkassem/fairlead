use std::collections::HashSet;

use fairlead_core::config::SizeRules;

use crate::finding::{Finding, Measure};
use crate::functions;
use crate::rules::{Preset, Scope, Source};

/// File length, and function length for files with a syntax tree.
pub(crate) struct Size {
    scope: Scope,
    file_lines: Option<u32>,
    function_lines: Option<u32>,
    test_hooks: HashSet<String>,
    ratchet: bool,
}

impl Size {
    pub(crate) fn new(config: &SizeRules) -> Result<Size, String> {
        Ok(Size {
            scope: Scope::new(config.files.items(), config.exclude.items())?,
            file_lines: config.file_lines,
            function_lines: config.function_lines,
            test_hooks: config.test_hooks.items().iter().cloned().collect(),
            ratchet: config.ratchet,
        })
    }
}

impl Preset for Size {
    fn ratcheted(&self) -> Vec<&'static str> {
        if self.ratchet {
            vec!["file-length", "function-length"]
        } else {
            Vec::new()
        }
    }

    fn applies(&self, path: &str) -> bool {
        self.scope.contains(path)
    }

    fn check(&self, source: &Source<'_>) -> Vec<Finding> {
        let mut found = Vec::new();
        if let Some(limit) = self.file_lines {
            found.extend(file_length(source, u64::from(limit)));
        }
        if let (Some(limit), Some(tree)) = (self.function_lines, source.tree()) {
            let limit = u64::from(limit);
            for f in functions::lengths(tree, source.text, &self.test_hooks) {
                if f.own > limit {
                    found.push(Finding {
                        file: source.path.to_string(),
                        line: f.line,
                        rule: "function-length",
                        message: format!(
                            "function's own lines are {}, over {limit} ({} total)",
                            f.own, f.total
                        ),
                        anchor: f.name,
                        measure: Some(Measure { size: f.own, limit }),
                    });
                }
            }
        }
        found
    }
}

/// A trailing newline doesn't start a line.
fn file_length(source: &Source<'_>, limit: u64) -> Option<Finding> {
    let lines = source.text.lines().count() as u64;
    (lines > limit).then(|| Finding {
        file: source.path.to_string(),
        line: 1,
        rule: "file-length",
        message: format!("file is {lines} lines, over {limit}"),
        anchor: String::new(),
        measure: Some(Measure { size: lines, limit }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(file_lines: Option<u32>, function_lines: Option<u32>) -> Size {
        Size::new(&SizeRules {
            files: vec!["**".to_string()].into(),
            file_lines,
            function_lines,
            ..SizeRules::default()
        })
        .unwrap()
    }

    fn check(size: &Size, path: &str, text: &str) -> Vec<Finding> {
        size.check(&Source::new(path, text))
    }

    #[test]
    fn a_file_at_the_limit_is_fine_and_one_line_over_is_a_finding() {
        assert!(check(&size(Some(2), None), "a.ts", "a\nb\n").is_empty());
        let found = check(&size(Some(2), None), "a.ts", "a\nb\nc\n");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].to_string(),
            "a.ts:1 file-length: file is 3 lines, over 2"
        );
        assert_eq!(found[0].measure, Some(Measure { size: 3, limit: 2 }));
    }

    #[test]
    fn a_missing_final_newline_and_a_blank_last_line_count_as_lines_do() {
        assert!(check(&size(Some(2), None), "a.ts", "a\nb").is_empty());
        assert_eq!(check(&size(Some(2), None), "a.ts", "a\nb\n\n").len(), 1);
    }

    #[test]
    fn a_long_function_is_named_and_measured_and_a_short_one_is_not() {
        let text = "export function long() {\n  a()\n  b()\n}\nfunction short() {}\n";
        let found = check(&size(None, Some(3)), "a.ts", text);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].to_string(),
            "a.ts:1 function-length: function's own lines are 4, over 3 (4 total)"
        );
        assert_eq!(found[0].anchor, "long");
    }

    #[test]
    fn function_length_needs_a_syntax_tree() {
        let text = "a\nb\nc\nd\ne\n";
        assert!(check(&size(None, Some(1)), "a.sql", text).is_empty());
    }
}
