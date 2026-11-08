//! What the ledger reads from the GitHub API while a pull request is red:
//! a run and its attempts, the runs on the same commit, the base branch's
//! recent runs of the same workflow, and every workflow's recent runs on
//! the default branch. Rows are the dataset's own, so the ledger reads them
//! with replay's extractors; logs are fetched only for failed jobs that
//! matter, so a ledger costs tens of requests, not hundreds.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::dataset::{Job, Row};
use crate::fetch::{get, job_of};
use crate::github::Http;
use crate::window::seconds_between;

const PER_PAGE: usize = 100;

fn str_of<'v>(v: &'v Value, key: &str) -> &'v str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn u64_of(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Only names a ref or workflow GitHub would accept in a query.
fn plain(s: &str) -> Result<&str, String> {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
    {
        Ok(s)
    } else {
        Err(format!("`{s}` isn't a plain name"))
    }
}

/// A run's conclusion as a row keeps it: red, green or cancelled.
fn conclusion(run: &Value) -> &'static str {
    match str_of(run, "conclusion") {
        "failure" | "timed_out" | "startup_failure" => "failure",
        "cancelled" | "skipped" | "stale" => "cancelled",
        "" => "in_progress",
        _ => "success",
    }
}

/// A row for one attempt of `run` with `jobs`.
pub fn row_of(repo: &str, run: &Value, attempt: u32, jobs: Vec<Job>) -> Row {
    let pr = run
        .get("pull_requests")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|p| p.get("number"))
        .and_then(Value::as_u64);
    let conclusion = if jobs.iter().any(Job::failed) {
        "failure"
    } else {
        conclusion(run)
    };
    Row {
        repo: repo.to_string(),
        run_id: u64_of(run, "id"),
        attempt,
        event: str_of(run, "event").to_string(),
        workflow: str_of(run, "name").to_string(),
        pr,
        head_sha: str_of(run, "head_sha").to_string(),
        base_sha: None,
        created_at: str_of(run, "created_at").to_string(),
        conclusion: conclusion.to_string(),
        seconds: seconds_between(str_of(run, "run_started_at"), str_of(run, "updated_at")),
        jobs,
    }
}

/// One run, by id.
pub fn run(http: &dyn Http, repo: &str, id: u64) -> Result<Value, String> {
    get(http, &format!("/repos/{repo}/actions/runs/{id}"))
}

/// The jobs of one attempt; a failed job named by `logs_for` (all, when
/// it's `None`) comes with its steps, annotations and log excerpt.
pub fn jobs(
    http: &dyn Http,
    repo: &str,
    run_id: u64,
    attempt: u32,
    logs_for: Option<&BTreeSet<String>>,
) -> Result<Vec<Job>, String> {
    let body = get(
        http,
        &format!("/repos/{repo}/actions/runs/{run_id}/attempts/{attempt}/jobs?per_page={PER_PAGE}"),
    )?;
    body.get("jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|j| {
            let wanted = logs_for.is_none_or(|set| set.contains(str_of(j, "name")));
            if wanted && str_of(j, "conclusion") == "failure" {
                job_of(http, repo, j)
            } else {
                Ok(Job {
                    name: str_of(j, "name").to_string(),
                    conclusion: str_of(j, "conclusion").to_string(),
                    failed_steps: Vec::new(),
                    annotations: Vec::new(),
                    annotations_capped: false,
                    log: Vec::new(),
                    image: None,
                    seconds: seconds_between(str_of(j, "started_at"), str_of(j, "completed_at")),
                })
            }
        })
        .collect()
}

/// The newest run of each workflow and event on commit `sha`.
pub fn runs_on(http: &dyn Http, repo: &str, sha: &str) -> Result<Vec<Value>, String> {
    let body = get(
        http,
        &format!(
            "/repos/{repo}/actions/runs?head_sha={}&per_page={PER_PAGE}",
            plain(sha)?
        ),
    )?;
    let mut newest: BTreeMap<(u64, String), Value> = BTreeMap::new();
    for run in body
        .get("workflow_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = (u64_of(run, "workflow_id"), str_of(run, "event").to_string());
        let newer = newest
            .get(&key)
            .is_none_or(|n| str_of(n, "created_at") < str_of(run, "created_at"));
        if newer {
            newest.insert(key, run.clone());
        }
    }
    Ok(newest.into_values().collect())
}

/// Earlier attempts of `run`, and the other runs `on_commit` lists of the
/// same workflow, with each job's outcome and no logs: enough to see a job
/// that passed on the same tree.
pub fn same_tree(
    http: &dyn Http,
    repo: &str,
    run: &Value,
    on_commit: &[Value],
) -> Result<Vec<Row>, String> {
    let id = u64_of(run, "id");
    let last = u64_of(run, "run_attempt").max(1) as u32;
    let none = BTreeSet::new();
    let mut rows = Vec::new();
    for attempt in 1..last {
        rows.push(row_of(
            repo,
            run,
            attempt,
            jobs(http, repo, id, attempt, Some(&none))?,
        ));
    }
    for other in on_commit
        .iter()
        .filter(|o| u64_of(o, "id") != id && u64_of(o, "workflow_id") == u64_of(run, "workflow_id"))
    {
        let attempt = u64_of(other, "run_attempt").max(1) as u32;
        let listed = jobs(http, repo, u64_of(other, "id"), attempt, Some(&none))?;
        rows.push(row_of(repo, other, attempt, listed));
    }
    Ok(rows)
}

/// The listing of a workflow's completed runs on `branch`, newest first,
/// with its total count; pull request and merge queue runs left out.
fn branch_listing(
    http: &dyn Http,
    repo: &str,
    workflow_id: u64,
    branch: &str,
    limit: usize,
) -> Result<(Vec<Value>, u64), String> {
    let body = get(
        http,
        &format!(
            "/repos/{repo}/actions/workflows/{workflow_id}/runs?branch={}&status=completed&per_page={}",
            plain(branch)?,
            limit.clamp(1, PER_PAGE)
        ),
    )?;
    let runs: Vec<Value> = body
        .get("workflow_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| {
            !matches!(
                str_of(r, "event"),
                "pull_request" | "pull_request_target" | "merge_group"
            )
        })
        .cloned()
        .collect();
    Ok((runs, u64_of(&body, "total_count")))
}

/// The base branch's newest `limit` completed runs of a workflow, newest
/// first, with logs for the failed jobs named in `jobs_failing`.
pub fn base_runs(
    http: &dyn Http,
    repo: &str,
    workflow_id: u64,
    branch: &str,
    limit: usize,
    jobs_failing: &BTreeSet<String>,
) -> Result<Vec<Row>, String> {
    let (listed, _) = branch_listing(http, repo, workflow_id, branch, limit)?;
    listed
        .iter()
        .filter(|r| conclusion(r) != "cancelled")
        .map(|r| {
            let attempt = u64_of(r, "run_attempt").max(1) as u32;
            let found = jobs(http, repo, u64_of(r, "id"), attempt, Some(jobs_failing))?;
            Ok(row_of(repo, r, attempt, found))
        })
        .collect()
}

/// The repository's default branch.
pub fn default_branch(http: &dyn Http, repo: &str) -> Result<String, String> {
    let body = get(http, &format!("/repos/{repo}"))?;
    Some(str_of(&body, "default_branch").to_string())
        .filter(|b| !b.is_empty())
        .ok_or_else(|| "the repository has no default branch".to_string())
}

/// Every active workflow's newest `limit` runs on `branch`, by event,
/// newest first; the newest two red runs of each carry their failed jobs'
/// logs, for drift. The flag says the runs are all the workflow has had.
pub fn branch_history(
    http: &dyn Http,
    repo: &str,
    branch: &str,
    limit: usize,
) -> Result<Vec<(Vec<Row>, bool)>, String> {
    let body = get(
        http,
        &format!("/repos/{repo}/actions/workflows?per_page={PER_PAGE}"),
    )?;
    let mut out = Vec::new();
    for workflow in body
        .get("workflows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|w| str_of(w, "state") == "active")
    {
        let (listed, total) = branch_listing(http, repo, u64_of(workflow, "id"), branch, limit)?;
        let whole = (total as usize) <= limit;
        let mut by_event: BTreeMap<String, Vec<Row>> = BTreeMap::new();
        for run in &listed {
            let rows = by_event
                .entry(str_of(run, "event").to_string())
                .or_default();
            let fetched = rows.iter().filter(|r| !r.jobs.is_empty()).count();
            let leading_red = fetched < 2 && !rows.iter().any(|r| r.conclusion == "success");
            let row = if leading_red && conclusion(run) == "failure" {
                let attempt = u64_of(run, "run_attempt").max(1) as u32;
                row_of(
                    repo,
                    run,
                    attempt,
                    jobs(http, repo, u64_of(run, "id"), attempt, None)?,
                )
            } else {
                row_of(
                    repo,
                    run,
                    u64_of(run, "run_attempt").max(1) as u32,
                    Vec::new(),
                )
            };
            rows.push(row);
        }
        out.extend(by_event.into_values().map(|rows| (rows, whole)));
    }
    Ok(out)
}
