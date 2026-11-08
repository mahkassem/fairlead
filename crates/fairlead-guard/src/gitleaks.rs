//! `preset = "gitleaks"`: gitleaks finds secrets in what files say, and
//! Fairlead only runs it and reads its JSON report. The report is written
//! with `--redact` and only the file, line, rule and fingerprint are read
//! from it, so a secret's value never reaches a message or a log.

use std::path::{Path, PathBuf};

use fairlead_core::config::Stage;
use serde::Deserialize;

use crate::finding::Finding;

pub const PROGRAM: &str = "gitleaks";
/// Said once when the binary isn't there; a missing scanner isn't a failure.
pub const MISSING: &str = "gitleaks not installed; secret content not scanned";

/// The fields of one entry in gitleaks' JSON report that Fairlead reads.
/// `Secret` and `Match` are left out on purpose.
#[derive(Debug, Deserialize)]
struct Leak {
    #[serde(rename = "RuleID")]
    rule: String,
    #[serde(rename = "File")]
    file: String,
    #[serde(rename = "StartLine", default)]
    line: u32,
    #[serde(rename = "Fingerprint", default)]
    fingerprint: String,
}

/// The arguments for a stage: the staged changes at commit, the files on
/// disk at check, which the caller narrows to tracked files.
pub fn args(stage: Stage, report: &Path) -> Vec<String> {
    let mut args: Vec<String> = match stage {
        Stage::Commit => vec!["git", "--pre-commit", "--staged"],
        _ => vec!["dir"],
    }
    .into_iter()
    .map(String::from)
    .collect();
    for a in [
        "--redact",
        "--no-banner",
        "--log-level",
        "error",
        "--exit-code",
        "0",
        "--report-format",
        "json",
        "--report-path",
    ] {
        args.push(a.into());
    }
    args.push(report.to_string_lossy().into_owned());
    args.push(".".into());
    args
}

/// What a run gave: findings, or a note to show when the binary is missing.
pub enum Ran {
    Found(Vec<Finding>),
    Missing,
}

fn report_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::env::temp_dir().join(format!(
        "fairlead-gitleaks-{}-{nanos}.json",
        std::process::id()
    ))
}

pub fn run(id: &'static str, root: &Path, stage: Stage) -> Result<Ran, String> {
    let report = report_path();
    let out = match fairlead_core::process::command(PROGRAM, root)
        .args(args(stage, &report))
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ran::Missing),
        Err(e) => return Err(format!("{id}: {PROGRAM}: {e}")),
    };
    let text = std::fs::read_to_string(&report);
    let _ = std::fs::remove_file(&report);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        return Err(format!(
            "{id}: gitleaks failed with {} ({last}); Fairlead runs gitleaks 8.19 or later",
            out.status
        ));
    }
    let text = text.map_err(|e| format!("{id}: gitleaks wrote no report: {e}"))?;
    parse(id, &text).map(Ran::Found)
}

/// Findings from a report, one per leak, with the value left out.
pub fn parse(id: &'static str, report: &str) -> Result<Vec<Finding>, String> {
    if report.trim().is_empty() {
        return Ok(Vec::new());
    }
    let leaks: Vec<Leak> = serde_json::from_str(report)
        .map_err(|e| format!("{id}: can't read gitleaks' report: {e}"))?;
    Ok(leaks
        .into_iter()
        .map(|l| {
            let file = l.file.strip_prefix("./").unwrap_or(&l.file).to_string();
            let ignore = if l.fingerprint.is_empty() {
                String::new()
            } else {
                format!(", or add `{}` to .gitleaksignore", l.fingerprint)
            };
            Finding {
                file,
                line: l.line.max(1),
                rule: id,
                message: format!(
                    "{}: a likely secret (value not shown); remove it and rotate it{ignore}",
                    l.rule
                ),
                anchor: l.rule,
                measure: None,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gitleaks 8.x's JSON report, with an obviously fake value in the
    /// fields a parser must never read.
    const REPORT: &str = r#"[
 {
  "RuleID": "generic-api-key",
  "Description": "Detected a Generic API Key.",
  "StartLine": 7,
  "EndLine": 7,
  "StartColumn": 3,
  "EndColumn": 40,
  "Match": "api_key = FAKE-VALUE-FOR-A-TEST",
  "Secret": "FAKE-VALUE-FOR-A-TEST",
  "File": "./config/app.php",
  "SymlinkFile": "",
  "Commit": "",
  "Entropy": 3.9,
  "Author": "",
  "Email": "",
  "Date": "",
  "Message": "",
  "Tags": [],
  "Fingerprint": "config/app.php:generic-api-key:7"
 }
]"#;

    #[test]
    fn a_report_becomes_file_line_findings_without_the_secret() {
        let found = parse("gitleaks", REPORT).unwrap();
        assert_eq!(found.len(), 1);
        let line = found[0].to_string();
        assert!(
            line.starts_with("config/app.php:7 gitleaks: generic-api-key:"),
            "{line}"
        );
        assert!(line.contains("config/app.php:generic-api-key:7"), "{line}");
        assert!(!line.contains("FAKE-VALUE"), "{line}");
        assert!(parse("gitleaks", "").unwrap().is_empty());
        assert!(parse("gitleaks", "[]").unwrap().is_empty());
        assert!(parse("gitleaks", "not json").is_err());
    }

    #[test]
    fn the_commit_stage_scans_staged_changes_and_the_check_stage_the_tree() {
        let report = Path::new("r.json");
        let commit = args(Stage::Commit, report).join(" ");
        assert!(
            commit.starts_with("git --pre-commit --staged --redact"),
            "{commit}"
        );
        let check = args(Stage::Check, report).join(" ");
        assert!(check.starts_with("dir --redact"), "{check}");
        assert!(
            check.ends_with("--report-format json --report-path r.json ."),
            "{check}"
        );
    }
}
