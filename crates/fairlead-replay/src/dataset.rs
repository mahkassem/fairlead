//! The replay dataset: one JSON line per completed CI run attempt, failed
//! or not, appended by `replay fetch` and never rewritten. A failed job keeps
//! the extractor's input (failure-level annotations and the log lines around
//! each failure), not its output, so a better extractor can re-read old rows
//! after the logs themselves have expired.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::extract::{bun_header, clean};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub repo: String,
    pub run_id: u64,
    pub attempt: u32,
    /// `pull_request` or `merge_group`.
    pub event: String,
    pub workflow: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u64>,
    pub head_sha: String,
    /// The base branch's commit the run's change was measured from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha: Option<String>,
    pub created_at: String,
    pub conclusion: String,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    pub conclusion: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// The annotations hit GitHub's per-job cap, so some may be missing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub annotations_capped: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub path: String,
    #[serde(default)]
    pub title: String,
}

impl Job {
    pub fn failed(&self) -> bool {
        self.conclusion == "failure"
    }
}

/// The log lines worth keeping for extraction: each `FAIL`, `●` or bun
/// `(fail)` line and the two after it, the bun file header a failure sits
/// under, and bun's summary line, which ends what the extractor reads. Capped
/// so one noisy job can't bloat the dataset.
pub fn log_excerpt(log: &str) -> Vec<String> {
    const AFTER: usize = 2;
    const CAP: usize = 400;
    let lines: Vec<&str> = log.lines().collect();
    let mut keep = BTreeSet::new();
    let mut header = None;
    for (i, line) in lines.iter().enumerate() {
        let cleaned = clean(line);
        if bun_header(&cleaned).is_some() {
            header = Some(i);
        }
        if line.contains("FAIL") || line.contains('●') || line.contains("(fail)") {
            keep.extend(i..(i + 1 + AFTER).min(lines.len()));
            keep.extend(header.filter(|_| line.contains("(fail)")));
        }
        if cleaned.trim_end().ends_with("failed:") {
            keep.insert(i);
        }
    }
    keep.into_iter()
        .take(CAP)
        .map(|i| lines[i].trim_end().to_string())
        .collect()
}

pub fn read(path: &Path) -> Result<Vec<Row>, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|e| format!("{}:{}: {e}", path.display(), i + 1))
        })
        .collect()
}

/// Appends the rows not already present by (run, attempt); returns how many.
pub fn append(path: &Path, rows: &[Row]) -> Result<usize, String> {
    let mut existing: BTreeSet<(u64, u32)> =
        read(path)?.iter().map(|r| (r.run_id, r.attempt)).collect();
    let fresh: Vec<&Row> = rows
        .iter()
        .filter(|r| existing.insert((r.run_id, r.attempt)))
        .collect();
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    for row in &fresh {
        let line = serde_json::to_string(row).expect("row serializes");
        writeln!(file, "{line}").map_err(|e| e.to_string())?;
    }
    Ok(fresh.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(run_id: u64, attempt: u32) -> Row {
        Row {
            repo: "o/r".into(),
            run_id,
            attempt,
            event: "pull_request".into(),
            workflow: "ci".into(),
            pr: Some(1),
            head_sha: "a".into(),
            base_sha: None,
            created_at: "2026-09-01T00:00:00Z".into(),
            conclusion: "failure".into(),
            jobs: Vec::new(),
        }
    }

    #[test]
    fn append_skips_rows_already_recorded() {
        let path =
            std::env::temp_dir().join(format!("fairlead-dataset-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert_eq!(append(&path, &[row(1, 1), row(1, 2)]).unwrap(), 2);
        assert_eq!(append(&path, &[row(1, 2), row(2, 1)]).unwrap(), 1);
        assert_eq!(read(&path).unwrap().len(), 3);
    }

    #[test]
    fn the_excerpt_keeps_failures_and_what_follows() {
        let log = "ok\n FAIL  a.test.ts > t\nError: x\nat y\nat z\nok\n";
        assert_eq!(
            log_excerpt(log),
            [" FAIL  a.test.ts > t", "Error: x", "at y"]
        );
    }
}
