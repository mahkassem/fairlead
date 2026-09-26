use crate::finding::{Finding, Measure};
use crate::rules::{Rule, Scope, Source};

/// A file over a number of lines. A trailing newline doesn't start a line.
pub(crate) struct FileLength {
    scope: Scope,
    limit: u32,
    ratchet: bool,
}

impl FileLength {
    pub(crate) fn new(scope: Scope, limit: u32, ratchet: bool) -> FileLength {
        FileLength {
            scope,
            limit,
            ratchet,
        }
    }
}

impl Rule for FileLength {
    fn id(&self) -> &'static str {
        "file-length"
    }

    fn ratcheted(&self) -> bool {
        self.ratchet
    }

    fn applies(&self, path: &str) -> bool {
        self.scope.contains(path)
    }

    fn check(&self, source: Source<'_>) -> Vec<Finding> {
        let lines = source.text.lines().count() as u64;
        let limit = u64::from(self.limit);
        if lines <= limit {
            return Vec::new();
        }
        vec![Finding {
            file: source.path.to_string(),
            line: 1,
            rule: self.id(),
            message: format!("file is {lines} lines, over {limit}"),
            anchor: String::new(),
            measure: Some(Measure { size: lines, limit }),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(limit: u32) -> FileLength {
        FileLength::new(Scope::new(&["**".to_string()], &[]).unwrap(), limit, true)
    }

    fn check(limit: u32, text: &str) -> Vec<Finding> {
        rule(limit).check(Source { path: "a.ts", text })
    }

    #[test]
    fn a_file_at_the_limit_is_fine_and_one_line_over_is_a_finding() {
        assert!(check(2, "a\nb\n").is_empty());
        let found = check(2, "a\nb\nc\n");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].to_string(),
            "a.ts:1 file-length: file is 3 lines, over 2"
        );
        assert_eq!(found[0].measure, Some(Measure { size: 3, limit: 2 }));
    }

    #[test]
    fn a_missing_final_newline_and_a_blank_last_line_count_as_lines_do() {
        assert!(check(2, "a\nb").is_empty());
        assert_eq!(check(2, "a\nb\n\n").len(), 1);
    }
}
