//! What a failed job's log says about why it failed, apart from which test:
//! a timeout with no assertion behind it, which a slow runner gives as
//! readily as a change, and the marks of infrastructure failing before or
//! around the tests (a lost runner, a refused bill, an out-of-memory kill,
//! a service answering 5xx).

use std::sync::OnceLock;

use regex::Regex;

use crate::dataset::Job;

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("built-in pattern compiles"))
}

/// A runner's own words for a test that ran out of time.
pub fn is_timeout(line: &str) -> bool {
    static TIMEOUT: OnceLock<Regex> = OnceLock::new();
    re(
        &TIMEOUT,
        r"(?i)\btimed out\b|exceeded timeout of|timeout of \d+ ?m?s exceeded|test timeout of \d+|\bTimeoutError\b|Failed: Timeout >",
    )
    .is_match(line)
}

/// A line that reports a failed assertion: the test ran and got a wrong answer.
pub fn is_assertion(line: &str) -> bool {
    static ASSERT: OnceLock<Regex> = OnceLock::new();
    re(
        &ASSERT,
        r"(?i)assert|\bexpected\b|\bexpect\(|\.to(?:Be|Equal|Match|Contain|Throw|HaveBeenCalled)|Failed asserting|\bmismatch\b",
    )
    .is_match(line)
}

/// A job whose failure is a timeout and nothing else: some line names a
/// timeout and none an assertion.
pub fn timed_out_without_assertion(log: &[String]) -> bool {
    log.iter().any(|l| is_timeout(l)) && !log.iter().any(|l| is_assertion(l))
}

/// The infrastructure marks, each with the words a verdict gives for it.
const INFRA: [(&str, &str); 8] = [
    (
        r"(?i)runner has received a shutdown signal|lost communication with the server|the runner .* (?:did not connect|stopped)",
        "the runner was lost",
    ),
    (
        r"(?i)recent account payments have failed|spending limit|billing|quota (?:exceeded|has been reached)",
        "a billing or quota refusal",
    ),
    (
        r"(?i)exit code 137\b|\bOOMKilled\b|out of memory|cannot allocate memory",
        "out of memory (exit 137)",
    ),
    (
        r"(?i)\b(?:500 internal server error|502 bad gateway|503 service (?:temporarily )?unavailable|504 gateway time-?out)\b|returned (?:a )?50[0-4]\b|status(?: code)?:? 50[0-4]\b",
        "a service answered 5xx",
    ),
    (r"(?i)no space left on device", "the runner ran out of disk"),
    (
        r"(?i)could not resolve host|temporary failure in name resolution|connection reset by peer|ETIMEDOUT|ECONNRESET",
        "the network failed",
    ),
    (
        r"(?i)the job (?:was not started|has exceeded the maximum execution time)",
        "the job hit the platform's limit",
    ),
    (r"(?i)rate limit exceeded|API rate limit", "a rate limit"),
];

fn infra_patterns() -> &'static Vec<(Regex, &'static str)> {
    static ALL: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    ALL.get_or_init(|| {
        INFRA
            .iter()
            .map(|(p, why)| (Regex::new(p).expect("built-in pattern compiles"), *why))
            .collect()
    })
}

/// The infrastructure mark on a line, if it carries one.
pub fn infra_mark(line: &str) -> Option<&'static str> {
    infra_patterns()
        .iter()
        .find(|(re, _)| re.is_match(line))
        .map(|(_, why)| *why)
}

/// A step that only prepares the job: nothing in it is the change's code.
pub fn is_setup_step(name: &str) -> bool {
    static SETUP: OnceLock<Regex> = OnceLock::new();
    re(
        &SETUP,
        r"(?i)^(?:set up job|post |complete job)|checkout|\binstall|setup|set up|\bcache\b|restore|download|login|log in|toolchain|dependencies|\bdeps\b",
    )
    .is_match(name)
}

/// Why a job failed for reasons outside the change, if it did: every
/// failing step prepares the job, or the log carries an infrastructure mark
/// and no assertion. `tests_named` says whether any failing test was found.
pub fn infra(job: &Job, tests_named: bool) -> Option<String> {
    let setup = !job.failed_steps.is_empty() && job.failed_steps.iter().all(|s| is_setup_step(s));
    if setup && !tests_named {
        return Some(format!(
            "failed before any test ran, in {}",
            job.failed_steps.join(", ")
        ));
    }
    if job.log.iter().any(|l| is_assertion(l)) {
        return None;
    }
    job.log
        .iter()
        .find_map(|l| infra_mark(l))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(steps: &[&str], log: &[&str]) -> Job {
        Job {
            name: "unit".into(),
            conclusion: "failure".into(),
            failed_steps: steps.iter().map(|s| s.to_string()).collect(),
            annotations: Vec::new(),
            annotations_capped: false,
            log: log.iter().map(|s| s.to_string()).collect(),
            image: None,
            seconds: None,
        }
    }

    #[test]
    fn a_timeout_with_no_assertion_is_told_from_one_with_an_assertion() {
        let only = [
            "FAIL  src/a.test.ts > waits",
            "Error: Test timed out in 5000ms.",
        ];
        let lines: Vec<String> = only.iter().map(|s| s.to_string()).collect();
        assert!(timed_out_without_assertion(&lines));
        let mut both = lines.clone();
        both.push("AssertionError: expected 1 to be 2".into());
        assert!(!timed_out_without_assertion(&both));
        assert!(is_timeout("panic: test timed out after 10m0s"));
        assert!(is_timeout(
            "thrown: \"Exceeded timeout of 5000 ms for a test."
        ));
        assert!(!is_timeout("expected 1 to be 2"));
    }

    #[test]
    fn a_job_that_failed_in_setup_or_with_an_infra_mark_is_infra() {
        let setup = job(&["Install dependencies"], &[]);
        assert!(infra(&setup, false)
            .unwrap()
            .contains("before any test ran"));
        assert_eq!(infra(&setup, true), None, "a named test means tests ran");
        let oom = job(
            &["Test"],
            &["##[error]Process completed with exit code 137."],
        );
        assert_eq!(
            infra(&oom, false).as_deref(),
            Some("out of memory (exit 137)")
        );
        let bad = job(
            &["Test"],
            &["Error: 503 Service Unavailable", "expected 200"],
        );
        assert_eq!(infra(&bad, false), None, "an assertion is the test's own");
        let lost = job(&[], &["The runner has received a shutdown signal."]);
        assert_eq!(infra(&lost, false).as_deref(), Some("the runner was lost"));
        assert_eq!(infra(&job(&["Test"], &["FAIL a"]), false), None);
    }
}
