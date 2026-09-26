use std::fmt;

/// One thing a rule found in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    pub line: u32,
    pub rule: &'static str,
    /// What is wrong, never where: the line is its own field, so the same
    /// finding on a moved line compares equal.
    pub message: String,
    /// What the finding is about, stable across edits elsewhere in the
    /// file: a comment's text, a function's name, or empty for the file.
    pub anchor: String,
    /// Set for rules that measure something against a limit.
    pub measure: Option<Measure>,
}

/// How big the thing is, and the limit it went over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measure {
    pub size: u64,
    pub limit: u64,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{} {}: {}",
            self.file, self.line, self.rule, self.message
        )
    }
}

/// Findings in reading order: by file, then line, then rule.
pub fn sort(findings: &mut [Finding]) {
    findings
        .sort_by(|a, b| (a.file.as_str(), a.line, a.rule).cmp(&(b.file.as_str(), b.line, b.rule)));
}
