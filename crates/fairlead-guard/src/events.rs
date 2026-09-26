//! What the guard decided, one JSON line per event in the git directory, so
//! it is never committed. An event carries rule ids, paths, counts and
//! timings; never file contents, comment text, commands, reasons or error
//! messages, since any of those can quote the code or a secret.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

const FILE: &str = "events.jsonl";
const OLD_FILE: &str = "events.1.jsonl";
const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Appends up to this size don't interleave when two hooks write at once.
const MAX_LINE: usize = 4096;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub at: String,
    /// `write`, `commit` or `check`.
    pub stage: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<usize>,
    /// `allow`, `deny`, `warn`, `error` or `timed_out`.
    pub decision: &'static str,
    pub rules: Vec<&'static str>,
    pub added: usize,
    pub ms: f64,
    pub fairlead: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

impl Event {
    pub fn new(stage: &'static str, decision: &'static str, elapsed: std::time::Duration) -> Event {
        Event {
            at: timestamp(SystemTime::now()),
            stage,
            event: None,
            tool: None,
            session: None,
            file: None,
            files: None,
            decision,
            rules: Vec::new(),
            added: 0,
            ms: (elapsed.as_secs_f64() * 10_000.0).round() / 10.0,
            fairlead: env!("CARGO_PKG_VERSION"),
            truncated: false,
        }
    }

    /// One line, under the size that appends atomically: a long path or
    /// rule list is dropped rather than split.
    fn line(&self) -> String {
        let text = serde_json::to_string(self).expect("events serialize");
        if text.len() < MAX_LINE {
            return text + "\n";
        }
        let short = Event {
            file: None,
            session: None,
            rules: Vec::new(),
            truncated: true,
            ..self.clone()
        };
        serde_json::to_string(&short).expect("events serialize") + "\n"
    }
}

pub struct EventLog {
    dir: PathBuf,
}

impl EventLog {
    /// The log for the repository at `root`, or none outside a repository.
    pub fn open(root: &Path) -> Option<EventLog> {
        fairlead_lang::cache::dir_for(root).map(EventLog::at)
    }

    pub fn at(dir: PathBuf) -> EventLog {
        EventLog { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE)
    }

    /// Appends one event, first moving a full log aside so one old file is kept.
    pub fn append(&self, event: &Event) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path();
        if std::fs::metadata(&path).is_ok_and(|m| m.len() >= MAX_BYTES) {
            std::fs::rename(&path, self.dir.join(OLD_FILE))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.write_all(event.line().as_bytes())
    }
}

/// RFC 3339 in UTC with milliseconds.
fn timestamp(at: SystemTime) -> String {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60,
        since.subsec_millis()
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date, by Howard
/// Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fairlead-events-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn timestamps_are_utc_with_milliseconds() {
        let at = UNIX_EPOCH + Duration::from_millis(1_790_450_651_221);
        assert_eq!(timestamp(at), "2026-09-26T19:24:11.221Z");
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn an_event_is_one_json_line_with_no_empty_fields() {
        let mut event = Event::new("commit", "deny", Duration::from_micros(7_440));
        event.rules = vec!["file-length"];
        event.added = 1;
        let line = event.line();
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["ms"], 7.4);
        assert_eq!(value["rules"][0], "file-length");
        assert!(value.get("file").is_none() && value.get("truncated").is_none());
    }

    #[test]
    fn an_event_too_long_to_append_whole_drops_its_path_and_rules() {
        let mut event = Event::new("write", "allow", Duration::ZERO);
        event.file = Some("a/".repeat(3000));
        event.rules = vec!["file-length"];
        let line = event.line();
        assert!(line.len() < MAX_LINE);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["truncated"], true);
        assert!(value.get("file").is_none());
    }

    #[test]
    fn appends_lines_and_moves_a_full_log_aside_keeping_one() {
        let dir = scratch("rotate");
        let log = EventLog::at(dir.clone());
        let event = Event::new("commit", "allow", Duration::ZERO);
        log.append(&event).unwrap();
        log.append(&event).unwrap();
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(text.lines().count(), 2);
        std::fs::write(log.path(), vec![b'x'; MAX_BYTES as usize]).unwrap();
        log.append(&event).unwrap();
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap().lines().count(),
            1
        );
        assert_eq!(
            std::fs::metadata(dir.join(OLD_FILE)).unwrap().len(),
            MAX_BYTES
        );
    }
}
