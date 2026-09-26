//! `replay fetch` against recorded API responses.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use fairlead_replay::fetch::{fetch, merge_queue_branch, Options, Stop};
use fairlead_replay::github::Http;
use serde_json::{json, Value};

struct Recorded {
    responses: BTreeMap<String, Value>,
    /// Paths answered with this status instead.
    status: BTreeMap<String, u16>,
    logs: BTreeMap<u64, String>,
    asked: Mutex<Vec<String>>,
}

impl Http for Recorded {
    fn get_json(&self, path: &str) -> Result<(u16, Value), String> {
        self.asked.lock().unwrap().push(path.to_string());
        let key = path.split('?').next().unwrap();
        if let Some(status) = self.status.get(key) {
            return Ok((*status, Value::Null));
        }
        Ok(self
            .responses
            .get(key)
            .map_or((404, Value::Null), |v| (200, v.clone())))
    }

    fn get_log(&self, _repo: &str, job_id: u64) -> Result<Option<String>, String> {
        Ok(self.logs.get(&job_id).cloned())
    }
}

fn api() -> Recorded {
    let mut responses = BTreeMap::new();
    let runs = |event: &str| {
        if event == "pull_request" {
            json!({ "total_count": 1, "workflow_runs": [
                { "id": 11, "name": "ci", "head_sha": "aaa", "head_branch": "feature", "run_attempt": 2, "created_at": "2026-09-20T10:00:00Z" }
            ]})
        } else {
            json!({ "total_count": 1, "workflow_runs": [
                { "id": 12, "name": "ci", "head_sha": "bbb", "head_branch": "gh-readonly-queue/main/pr-42-cafe1234", "run_attempt": 1, "created_at": "2026-09-21T10:00:00Z" }
            ]})
        }
    };
    responses.insert("/repos/o/r/actions/runs".into(), runs("pull_request"));
    responses.insert("/repos/o/r/actions/runs/11/attempts/1/jobs".into(), json!({ "jobs": [
        { "id": 101, "name": "test", "conclusion": "failure", "steps": [{ "name": "Run tests", "conclusion": "failure" }, { "name": "Setup", "conclusion": "success" }] },
        { "id": 102, "name": "lint", "conclusion": "success", "steps": [] }
    ]}));
    responses.insert(
        "/repos/o/r/actions/runs/11/attempts/2/jobs".into(),
        json!({ "jobs": [
            { "id": 103, "name": "test", "conclusion": "success", "steps": [] }
        ]}),
    );
    responses.insert(
        "/repos/o/r/actions/runs/12/attempts/1/jobs".into(),
        json!({ "jobs": [
            { "id": 104, "name": "test", "conclusion": "success", "steps": [] }
        ]}),
    );
    responses.insert("/repos/o/r/check-runs/101/annotations".into(), json!([
        { "path": "test/a.test.ts", "title": "a > works", "annotation_level": "failure" },
        { "path": ".github", "title": "Process completed with exit code 1.", "annotation_level": "failure" },
        { "path": "src/b.ts", "title": "unused", "annotation_level": "warning" }
    ]));
    responses.insert(
        "/repos/o/r/commits/aaa/pulls".into(),
        json!([{ "number": 7, "base": { "ref": "main" } }]),
    );
    let mut logs = BTreeMap::new();
    logs.insert(
        101,
        "setup\n FAIL  test/a.test.ts > a > works\nError: nope\n  at x\n  at y\ndone\n".to_string(),
    );
    Recorded {
        responses,
        status: BTreeMap::new(),
        logs,
        asked: Mutex::new(Vec::new()),
    }
}

#[test]
fn every_attempt_is_a_row_with_its_failures_pull_request_and_base() {
    let http = api();
    let opts = opts(None);
    let (rows, stop) = fetch(&http, &opts, &BTreeSet::new());
    assert_eq!(stop, Stop::Complete);
    // The runs listing ignores the event filter in this recording, so the
    // pull request run is listed under both events but recorded once.
    let mut keys: Vec<(u64, u32)> = rows.iter().map(|r| (r.run_id, r.attempt)).collect();
    let listed = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), listed, "{keys:?}");
    let first = rows
        .iter()
        .find(|r| r.run_id == 11 && r.attempt == 1)
        .unwrap();
    assert_eq!(first.conclusion, "failure");
    assert_eq!(first.pr, Some(7));
    let test = first.jobs.iter().find(|j| j.name == "test").unwrap();
    assert_eq!(test.failed_steps, ["Run tests"]);
    assert_eq!(test.annotations.len(), 1, "failure level only, no .github");
    assert_eq!(test.annotations[0].path, "test/a.test.ts");
    assert_eq!(
        test.log,
        [" FAIL  test/a.test.ts > a > works", "Error: nope", "  at x"]
    );
    let retry = rows
        .iter()
        .find(|r| r.run_id == 11 && r.attempt == 2)
        .unwrap();
    assert_eq!(retry.conclusion, "success");
}

#[test]
fn rows_already_recorded_are_skipped_before_their_jobs_are_fetched() {
    let http = api();
    let opts = opts(None);
    let seen: BTreeSet<(u64, u32)> = [(11, 1), (11, 2)].into_iter().collect();
    let (rows, _) = fetch(&http, &opts, &seen);
    assert!(rows.iter().all(|r| r.run_id != 11));
    assert!(http
        .asked
        .lock()
        .unwrap()
        .iter()
        .all(|p| !p.contains("/runs/11/attempts")));
}

#[test]
fn a_merge_queue_branch_names_its_pull_request_and_base() {
    assert_eq!(
        merge_queue_branch("gh-readonly-queue/main/pr-42-cafe1234"),
        Some((42, "cafe1234".into()))
    );
    assert_eq!(
        merge_queue_branch("gh-readonly-queue/release/v2/pr-7-abc"),
        Some((7, "abc".into()))
    );
    assert_eq!(merge_queue_branch("feature/x"), None);
}

fn opts(limit: Option<usize>) -> Options<'static> {
    Options {
        repo: "o/r",
        since: "2026-09-01",
        clone: None,
        limit,
        workflows: Vec::new(),
        until: Some("2026-09-30"),
    }
}

#[test]
fn a_purged_check_run_reads_as_no_annotations() {
    let mut http = api();
    http.status
        .insert("/repos/o/r/check-runs/101/annotations".into(), 404);
    let (rows, stop) = fetch(&http, &opts(None), &BTreeSet::new());
    assert_eq!(stop, Stop::Complete);
    let first = rows
        .iter()
        .find(|r| r.run_id == 11 && r.attempt == 1)
        .unwrap();
    let test = first.jobs.iter().find(|j| j.name == "test").unwrap();
    assert!(test.annotations.is_empty());
    assert_eq!(test.log.len(), 3, "the log is still read");
}

#[test]
fn a_rate_limit_stops_the_fetch_before_writing_a_row_without_its_pull_request() {
    let mut http = api();
    http.status
        .insert("/repos/o/r/commits/aaa/pulls".into(), 403);
    let (rows, stop) = fetch(&http, &opts(None), &BTreeSet::new());
    assert!(matches!(stop, Stop::Error(e) if e.contains("403")));
    assert!(rows
        .iter()
        .all(|r| r.pr.is_some() || r.event == "merge_group"));
}

#[test]
fn a_limit_stops_after_that_many_attempts() {
    let (rows, stop) = fetch(&api(), &opts(Some(1)), &BTreeSet::new());
    assert_eq!(stop, Stop::Limit);
    assert_eq!(rows.len(), 1);
}

#[test]
fn the_window_is_listed_a_week_at_a_time() {
    let http = api();
    let _ = fetch(&http, &opts(None), &BTreeSet::new());
    let asked = http.asked.lock().unwrap();
    let ranges: BTreeSet<&str> = asked
        .iter()
        .filter(|p| p.contains("event=pull_request"))
        .filter_map(|p| p.split("created=").nth(1))
        .map(|rest| rest.split('&').next().unwrap())
        .collect();
    let expected: BTreeSet<&str> = [
        "2026-09-01..2026-09-07",
        "2026-09-08..2026-09-14",
        "2026-09-15..2026-09-21",
        "2026-09-22..2026-09-28",
        "2026-09-29..2026-09-30",
    ]
    .into_iter()
    .collect();
    assert_eq!(ranges, expected);
}

#[test]
fn only_named_workflows_and_runs_that_ran_are_recorded() {
    let mut http = api();
    http.responses.insert(
        "/repos/o/r/actions/runs".into(),
        json!({ "total_count": 3, "workflow_runs": [
            { "id": 11, "name": "ci", "head_sha": "aaa", "head_branch": "feature", "run_attempt": 2, "created_at": "2026-09-20T10:00:00Z", "conclusion": "success" },
            { "id": 21, "name": "docs", "head_sha": "aaa", "head_branch": "feature", "run_attempt": 1, "created_at": "2026-09-20T10:00:00Z", "conclusion": "failure" },
            { "id": 31, "name": "ci", "head_sha": "ccc", "head_branch": "feature", "run_attempt": 1, "created_at": "2026-09-20T11:00:00Z", "conclusion": "cancelled" }
        ]}),
    );
    let mut o = opts(None);
    o.workflows = vec!["ci".into()];
    let (rows, stop) = fetch(&http, &o, &BTreeSet::new());
    assert_eq!(stop, Stop::Complete);
    let ids: BTreeSet<u64> = rows.iter().map(|r| r.run_id).collect();
    assert_eq!(ids, [11].into_iter().collect());
}
