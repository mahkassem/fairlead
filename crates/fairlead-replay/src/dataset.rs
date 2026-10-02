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
    /// The runner image and its version, `ubuntu-24.04 20260907.1`, from a
    /// failed job's log header; none for a passing job or a self-hosted runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
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
/// under, and bun's summary line, which ends what the extractor reads.
/// PHPUnit's section headers and numbered failures, and Pest's `FAILED`,
/// keep the frames after them. Surefire's `<<< ERROR!` lines and closing
/// lists, the Gradle task header a `FAILED` line sits under, and Gradle's
/// failed-task lines are kept too. Capped so one noisy job can't bloat the dataset.
pub fn log_excerpt(log: &str) -> Vec<String> {
    const AFTER: usize = 2;
    const CAP: usize = 400;
    let lines: Vec<&str> = log.lines().collect();
    let mut keep = BTreeSet::new();
    let mut header = None;
    let mut frames_until = 0;
    let mut task = None;
    let mut surefire_list = false;
    for (i, line) in lines.iter().enumerate() {
        let cleaned = clean(line);
        if bun_header(&cleaned).is_some() {
            header = Some(i);
        }
        if cleaned.starts_with("> Task :") {
            task = Some(i);
        }
        if cleaned.contains(" FAILED") && !cleaned.starts_with(char::is_whitespace) {
            keep.extend(task);
        }
        surefire_list = match cleaned.trim_end() {
            "[ERROR] Failures:" | "[ERROR] Errors:" => true,
            l => surefire_list && crate::jvm::in_surefire_list(l),
        };
        if surefire_list || jvm_failure(&cleaned) {
            keep.insert(i);
        }
        let unhandled = cleaned.trim() == "# Unhandled error between tests";
        let pytest_error =
            cleaned.trim_start().starts_with("ERROR ") || cleaned.contains(" ERROR collecting ");
        if line.contains("FAIL")
            || line.contains('●')
            || line.contains("(fail)")
            || unhandled
            || pytest_error
        {
            keep.extend(i..(i + 1 + AFTER).min(lines.len()));
            keep.extend(header.filter(|_| line.contains("(fail)") || unhandled));
        }
        if cleaned.trim_end().ends_with("failed:") {
            keep.insert(i);
        }
        if let Some(name) = cleaned.trim_start().strip_prefix("--- FAIL: ") {
            keep.extend(go_run_lines(
                &lines,
                i,
                name.split_whitespace().next().unwrap_or(""),
            ));
        }
        if go_file_line(&cleaned) {
            keep.insert(i);
        }
        if php_failure(&cleaned) {
            keep.insert(i);
            frames_until = i + PHP_FRAMES;
        } else if i <= frames_until && is_php_frame(&cleaned) {
            keep.insert(i);
        }
    }
    keep.into_iter()
        .take(CAP)
        .map(|i| lines[i].trim_end().to_string())
        .collect()
}

/// Under `go test -v` a test's log lines come before its `--- FAIL`: the
/// lines back to its `=== RUN` that name a test file, and that line.
fn go_run_lines(lines: &[&str], at: usize, name: &str) -> Vec<usize> {
    let top = name.split('/').next().unwrap_or(name);
    let mut out = Vec::new();
    for j in (at.saturating_sub(50)..at).rev() {
        let line = clean(lines[j]);
        let run = line
            .strip_prefix("=== RUN")
            .or_else(|| line.strip_prefix("=== CONT"))
            .map(str::trim);
        if run == Some(top) {
            out.push(j);
            break;
        }
        if run.is_some_and(|r| r.split('/').next() == Some(top)) || is_go_logged(&line) {
            out.push(j);
        }
    }
    out
}

fn is_go_logged(line: &str) -> bool {
    line.starts_with(char::is_whitespace)
        && line
            .trim_start()
            .split_once(':')
            .is_some_and(|(file, rest)| {
                file.ends_with("_test.go") && rest.starts_with(|c: char| c.is_ascii_digit())
            })
}

/// A panic's frame in a test file, or a compile error in one.
fn go_file_line(line: &str) -> bool {
    let t = line.trim();
    let Some((path, rest)) = t.split_once("_test.go:") else {
        return false;
    };
    !path.contains(' ')
        && rest.starts_with(|c: char| c.is_ascii_digit())
        && (rest.contains(" +0x")
            || rest.chars().all(|c| c.is_ascii_digit())
            || !line.starts_with(char::is_whitespace))
}

/// A Surefire test that errored, or a Gradle task's failure and its report.
fn jvm_failure(line: &str) -> bool {
    line.contains("<<< ERROR!")
        || line.contains("Execution failed for task '")
        || line.contains("There were failing tests. See the report at")
}

/// How far after a PHP failure its frames are kept.
const PHP_FRAMES: usize = 400;

/// A PHPUnit section header or numbered failure, or a Pest `FAILED` line.
fn php_failure(line: &str) -> bool {
    static FAILURE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    FAILURE
        .get_or_init(|| {
            regex::Regex::new(
                r"^\s*(?:There (?:was|were) \d+ [\w ]+:|\d+\) [\w\\]+::\w|FAILED\s+[\w\\]+ > )",
            )
            .expect("built-in pattern compiles")
        })
        .is_match(line)
}

fn is_php_frame(line: &str) -> bool {
    let line = line.trim();
    let line = line.strip_prefix("at ").unwrap_or(line);
    let line = line
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start();
    line.rsplit_once(".php:").is_some_and(|(path, n)| {
        !path.contains(' ') && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
    })
}

/// A hosted runner's image and version, as its log's "Runner Image" group
/// prints them near the top.
pub fn runner_image(log: &str) -> Option<String> {
    let mut image = None;
    for line in log.lines().take(120).map(clean) {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("Image: ") {
            image = Some(name.trim().to_string());
        } else if let (Some(name), Some(version)) = (&image, line.strip_prefix("Version: ")) {
            return Some(format!("{name} {}", version.trim()));
        }
    }
    None
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
