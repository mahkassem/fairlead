//! `[[guard.external]]`: another tool's findings, read from its output as
//! `file:line message` or `file:line:column message`, one per line.

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use fairlead_core::config::{ExternalRule, Stage};
use regex::Regex;

use crate::finding::Finding;

pub struct External {
    pub id: &'static str,
    command: Vec<String>,
    stages: Vec<Stage>,
    pub ratchet: bool,
}

impl External {
    /// The id is kept for the life of the process, like a built-in rule's.
    pub fn new(c: &ExternalRule) -> External {
        External {
            id: Box::leak(c.id.clone().into_boxed_str()),
            command: c.command.clone(),
            stages: c.stages.clone(),
            ratchet: c.ratchet,
        }
    }

    pub fn runs_at(&self, stage: Stage) -> bool {
        self.stages.contains(&stage)
    }

    /// Runs the command in `root` with `{files}` expanded to `files`. A
    /// non-zero exit is a failure only when it printed no findings, since a
    /// linter exits non-zero when it finds something.
    pub fn run(&self, root: &Path, files: &[String]) -> Result<Vec<Finding>, String> {
        let argv: Vec<String> = self
            .command
            .iter()
            .flat_map(|a| {
                if a == "{files}" {
                    files.to_vec()
                } else {
                    vec![a.clone()]
                }
            })
            .collect();
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| format!("{}: the command is empty", self.id))?;
        let out = Command::new(program)
            .args(args)
            .current_dir(root)
            .output()
            .map_err(|e| format!("{}: {program}: {e}", self.id))?;
        let found = parse(self.id, &String::from_utf8_lossy(&out.stdout));
        if !out.status.success() && found.is_empty() {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            return Err(format!(
                "{}: `{program}` failed with {} and printed no findings: {last}",
                self.id, out.status
            ));
        }
        Ok(found)
    }
}

fn line_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"^(?:\./)?(?P<file>[^\s:][^:]*):(?P<line>[0-9]+)(?::[0-9]+)?:? +(?P<message>\S.*)$",
        )
        .expect("valid")
    })
}

pub(crate) fn parse(id: &'static str, output: &str) -> Vec<Finding> {
    output
        .lines()
        .filter_map(|l| line_pattern().captures(l.trim_end()))
        .filter_map(|c| {
            Some(Finding {
                file: c["file"].to_string(),
                line: c["line"].parse().ok()?,
                rule: id,
                message: c["message"].to_string(),
                anchor: c["message"].to_string(),
                measure: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_are_read_with_or_without_a_column_and_other_lines_are_ignored() {
        let out = "src/a.ts:3 no-raw-color: use a token\n./src/b.tsx:10:4: something\nsummary: 2 problems\n\nnot a finding\n";
        let found: Vec<String> = parse("design", out).iter().map(|f| f.to_string()).collect();
        assert_eq!(
            found,
            [
                "src/a.ts:3 design: no-raw-color: use a token",
                "src/b.tsx:10 design: something"
            ]
        );
    }

    #[test]
    fn a_failing_command_with_no_findings_is_an_error_and_one_with_findings_is_not() {
        let rule = |command: &[&str]| {
            External::new(&ExternalRule {
                id: "x".into(),
                command: command.iter().map(|s| s.to_string()).collect(),
                stages: vec![Stage::Check],
                ratchet: false,
            })
        };
        let root = std::env::temp_dir();
        if cfg!(windows) {
            return;
        }
        assert!(rule(&["sh", "-c", "exit 3"]).run(&root, &[]).is_err());
        let found = rule(&["sh", "-c", "echo 'a.ts:1 bad' ; exit 1"])
            .run(&root, &[])
            .unwrap();
        assert_eq!(found.len(), 1);
        let files = rule(&["echo", "{files}"]).run(&root, &["a:1 x".into()]);
        assert_eq!(files.unwrap()[0].message, "x");
    }
}
