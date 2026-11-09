//! A failure's fingerprint: the job, the test and the failure's message
//! with what changes from run to run taken out (times, durations, ids,
//! ports, temporary paths, line and column numbers). Two runs that fail
//! alike give the same fingerprint, so a failure can be matched against the
//! base branch's history, and a red job whose fingerprint moved is a cause
//! that drifted.

use std::sync::OnceLock;

use regex::Regex;

use crate::extract::{clean, Printed};

/// The longest message kept, in characters.
const MESSAGE_CHARS: usize = 200;

fn rules() -> &'static Vec<(Regex, &'static str)> {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (r"\d{4}-\d\d-\d\d[T ]\d\d:\d\d(?::\d\d)?(?:\.\d+)?(?:Z|[+-]\d\d:?\d\d)?", "<time>"),
            (r"\b\d\d:\d\d:\d\d(?:\.\d+)?\b", "<time>"),
            (r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b", "<uuid>"),
            (r"(?i)\b0x[0-9a-f]+\b", "<hex>"),
            (
                r#"(?:/private)?/var/folders/[^\s'":]+|/tmp/[^\s'":]+|/home/runner/work/_temp/[^\s'":]+|[A-Za-z]:\\Users\\[^\\\s]+\\AppData\\Local\\Temp\\[^\s'":]+"#,
                "<tmp>",
            ),
            (r"(?i)\b(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\]):\d{2,5}\b", "$1:<port>"),
            (r"(?i)\bport \d{2,5}\b", "port <port>"),
            (r"(<tmp>|\.[A-Za-z]{1,5}):\d+(?::\d+)?\b", "$1:<line>"),
            (r"(?i)\b\d+(?:\.\d+)?\s?(?:ms|s|sec|secs|seconds?|m|mins?|minutes?)\b", "<duration>"),
            (r"\s+", " "),
        ]
        .into_iter()
        .map(|(p, to)| (Regex::new(p).expect("built-in pattern compiles"), to))
        .collect()
    })
}

/// `message` with run-to-run noise replaced by placeholders.
pub fn normalize(message: &str) -> String {
    let mut text = clean(message);
    for (re, to) in rules() {
        text = re.replace_all(&text, *to).into_owned();
    }
    // A long hex word with a digit in it is an id or a hash, not words.
    static HEX: OnceLock<Regex> = OnceLock::new();
    let hex =
        HEX.get_or_init(|| Regex::new(r"\b[0-9a-fA-F]{7,}\b").expect("built-in pattern compiles"));
    let text = hex.replace_all(&text, |c: &regex::Captures| {
        if c[0].bytes().any(|b| b.is_ascii_digit()) {
            "<hex>".to_string()
        } else {
            c[0].to_string()
        }
    });
    let text = text.trim();
    match text.char_indices().nth(MESSAGE_CHARS) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

/// FNV-1a over the parts, as 16 hex digits: stable across builds and platforms.
pub fn id(job: &str, test: &str, message: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in [job, "\u{1f}", test, "\u{1f}", message]
        .iter()
        .flat_map(|s| s.bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The test a printed failure names, as the ledger shows it.
pub fn test_id(p: &Printed) -> String {
    let path = match &p.project {
        Some(project) => format!("{project}:{}", p.path),
        None => p.path.clone(),
    };
    match &p.title {
        Some(title) => format!("{path} > {title}"),
        None => path,
    }
}

/// Words that mark a line as the failure itself rather than its context.
fn errorish(line: &str) -> bool {
    static ERROR: OnceLock<Regex> = OnceLock::new();
    ERROR
        .get_or_init(|| {
            Regex::new(r"(?i)error|assert|expected|exception|panic|timed out|timeout|failed|fatal|refused|denied|not found|exit code")
                .expect("built-in pattern compiles")
        })
        .is_match(line)
}

fn header(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("FAIL") || t.starts_with("(fail)") || t.starts_with("--- FAIL") || t.is_empty()
}

/// The message for a failing test: the first line after the one that names
/// it that reads like an error, or failing that the first line after it;
/// with no line naming it, the log's first error line.
pub fn message_for(log: &[String], printed: Option<&Printed>) -> String {
    let named = printed.and_then(|p| {
        let key = p.title.as_deref().unwrap_or(&p.path);
        log.iter()
            .position(|l| l.contains(key) || l.contains(&p.path))
    });
    if let Some(at) = named {
        let after: Vec<&String> = log.iter().skip(at + 1).take(6).collect();
        if let Some(line) = after
            .iter()
            .take_while(|l| !l.contains(" FAIL ") && !l.trim_start().starts_with("FAIL"))
            .find(|l| errorish(l) && !header(l))
            .or_else(|| after.iter().find(|l| !header(l)))
        {
            return normalize(line);
        }
    }
    log.iter()
        .find(|l| errorish(l) && !header(l))
        .map_or_else(String::new, |l| normalize(l))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_that_changes_between_runs_is_taken_out() {
        let a = normalize(
            "2026-11-07T10:00:01.123Z connect ECONNREFUSED 127.0.0.1:54321 after 1503ms at /tmp/vitest-x81/a.ts:12:7 id 3f2a9c1d-0b1e-4c2f-9a7d-5e6f7a8b9c0d ptr 0x7ffde",
        );
        let b = normalize(
            "2026-11-08T23:59:59Z connect ECONNREFUSED 127.0.0.1:40112 after 2 s at /tmp/vitest-q02/a.ts:14:1 id 0b1e3f2a-9c1d-4c2f-9a7d-aaaaaaaaaaaa ptr 0x1",
        );
        assert_eq!(a, b);
        assert!(a.contains("127.0.0.1:<port>") && a.contains("<tmp>") && a.contains("<uuid>"));
        assert_eq!(
            normalize("expected 3 to be 4 at src/a.test.ts:10:5 (deadbeef12)"),
            "expected 3 to be 4 at src/a.test.ts:<line> (<hex>)"
        );
    }

    #[test]
    fn the_same_parts_give_the_same_id_and_any_change_another() {
        assert_eq!(id("unit", "a > b", "m"), id("unit", "a > b", "m"));
        assert_ne!(id("unit", "a > b", "m"), id("e2e", "a > b", "m"));
        assert_ne!(id("unit", "ab", ""), id("unit", "a", "b"));
        assert_eq!(id("unit", "t", "m").len(), 16);
    }

    #[test]
    fn the_message_is_the_error_line_under_the_test() {
        let log: Vec<String> = [
            " FAIL  src/a.test.ts > adds",
            "",
            "AssertionError: expected 1 to be 2 // Object.is equality",
            " FAIL  src/b.test.ts > subtracts",
            "TypeError: x is undefined",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let p = |path: &str, title: &str| Printed {
            path: path.into(),
            project: None,
            title: Some(title.into()),
        };
        assert_eq!(
            message_for(&log, Some(&p("src/a.test.ts", "adds"))),
            "AssertionError: expected 1 to be 2 // Object.is equality"
        );
        assert_eq!(
            message_for(&log, Some(&p("src/b.test.ts", "subtracts"))),
            "TypeError: x is undefined"
        );
        assert_eq!(
            message_for(&log, None),
            "AssertionError: expected 1 to be 2 // Object.is equality"
        );
        assert_eq!(test_id(&p("src/a.test.ts", "adds")), "src/a.test.ts > adds");
    }
}
