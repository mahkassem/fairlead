//! `replay fetch`: every completed pull request and merge queue run in the
//! window, each attempt as its own row, so a job that failed and then passed
//! on a re-run is visible as flaky. Failed jobs keep their failure-level
//! annotations and log excerpts; rows already recorded are skipped before
//! any of their jobs are fetched, so weekly runs stay incremental.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::dataset::{log_excerpt, Annotation, Job, Row};
use crate::github::Http;

const PER_PAGE: usize = 100;
/// GitHub stops adding annotations past a per-step limit; a job with this
/// many may be missing some.
const ANNOTATION_CAP: usize = 10;
const EVENTS: [&str; 2] = ["pull_request", "merge_group"];

pub struct Options<'a> {
    pub repo: &'a str,
    /// The first day to list, `YYYY-MM-DD`.
    pub since: &'a str,
    /// A clone to fetch each head into and read base commits from.
    pub clone: Option<&'a Path>,
    /// Stop after this many run attempts, for a partial fetch.
    pub limit: Option<usize>,
}

fn get(http: &dyn Http, path: &str) -> Result<Value, String> {
    let (status, body) = http.get_json(path)?;
    match status {
        200 => Ok(body),
        403 | 429 => Err(format!(
            "GitHub refused {path} with {status} (rate limit or permissions); rows so far are kept"
        )),
        _ => Err(format!("GitHub answered {status} for {path}")),
    }
}

fn str_of<'v>(v: &'v Value, key: &str) -> &'v str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn git(clone: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `gh-readonly-queue/<base>/pr-<N>-<base sha>`: the queue's PR and base.
pub fn merge_queue_branch(branch: &str) -> Option<(u64, String)> {
    let rest = branch.strip_prefix("gh-readonly-queue/")?;
    let (_, tail) = rest.rsplit_once("/pr-")?;
    let (number, sha) = tail.split_once('-')?;
    Some((number.parse().ok()?, sha.to_string()))
}

/// The pull request a head commit belongs to, and its base branch.
fn pull_for(http: &dyn Http, repo: &str, sha: &str) -> Option<(u64, String)> {
    let pulls = get(http, &format!("/repos/{repo}/commits/{sha}/pulls")).ok()?;
    let first = pulls.as_array()?.first()?;
    Some((
        first.get("number")?.as_u64()?,
        first.get("base")?.get("ref")?.as_str()?.to_string(),
    ))
}

/// The base branch's first-parent commit when the run started.
fn base_at(clone: &Path, branch: &str, created_at: &str) -> Option<String> {
    git(clone, &["fetch", "-q", "origin", branch])?;
    git(
        clone,
        &[
            "rev-list",
            "-1",
            "--first-parent",
            &format!("--before={created_at}"),
            &format!("origin/{branch}"),
        ],
    )
    .filter(|s| !s.is_empty())
}

fn job_of(http: &dyn Http, repo: &str, job: &Value) -> Result<Job, String> {
    let conclusion = str_of(job, "conclusion").to_string();
    let mut out = Job {
        name: str_of(job, "name").to_string(),
        conclusion,
        failed_steps: Vec::new(),
        annotations: Vec::new(),
        annotations_capped: false,
        log: Vec::new(),
    };
    if !out.failed() {
        return Ok(out);
    }
    out.failed_steps = job
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| str_of(s, "conclusion") == "failure")
        .map(|s| str_of(s, "name").to_string())
        .collect();
    let id = job.get("id").and_then(Value::as_u64).unwrap_or(0);
    let notes = get(
        http,
        &format!("/repos/{repo}/check-runs/{id}/annotations?per_page=50"),
    )?;
    let notes = notes.as_array().cloned().unwrap_or_default();
    out.annotations_capped = notes.len() >= ANNOTATION_CAP;
    out.annotations = notes
        .iter()
        .filter(|n| {
            str_of(n, "annotation_level") == "failure" && !str_of(n, "path").starts_with(".github")
        })
        .map(|n| Annotation {
            path: str_of(n, "path").to_string(),
            title: str_of(n, "title").to_string(),
        })
        .collect();
    if let Some(log) = http.get_log(repo, id)? {
        out.log = log_excerpt(&log);
    }
    Ok(out)
}

fn runs(http: &dyn Http, repo: &str, event: &str, since: &str) -> Result<Vec<Value>, String> {
    let mut all = Vec::new();
    for page in 1.. {
        let path = format!("/repos/{repo}/actions/runs?event={event}&status=completed&created=%3E%3D{since}&per_page={PER_PAGE}&page={page}");
        let body = get(http, &path)?;
        let batch = body
            .get("workflow_runs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let total = body.get("total_count").and_then(Value::as_u64).unwrap_or(0) as usize;
        let done = batch.len() < PER_PAGE || page * PER_PAGE >= total;
        all.extend(batch);
        if done {
            break;
        }
    }
    Ok(all)
}

/// New rows for the runs in the window; `seen` holds (run, attempt) pairs
/// already recorded. On an API error the rows gathered so far come back
/// with it, so a partial fetch is never lost.
pub fn fetch(
    http: &dyn Http,
    opts: &Options,
    seen: &BTreeSet<(u64, u32)>,
) -> (Vec<Row>, Option<String>) {
    let mut rows = Vec::new();
    let mut pulls: BTreeMap<String, Option<(u64, String)>> = BTreeMap::new();
    for event in EVENTS {
        let listed = match runs(http, opts.repo, event, opts.since) {
            Ok(r) => r,
            Err(e) => return (rows, Some(e)),
        };
        for run in listed {
            let id = run.get("id").and_then(Value::as_u64).unwrap_or(0);
            let attempts = run.get("run_attempt").and_then(Value::as_u64).unwrap_or(1) as u32;
            for attempt in 1..=attempts {
                if seen.contains(&(id, attempt)) {
                    continue;
                }
                if opts.limit.is_some_and(|l| rows.len() >= l) {
                    return (rows, None);
                }
                match row_of(http, opts, &run, event, attempt, &mut pulls) {
                    Ok(row) => rows.push(row),
                    Err(e) => return (rows, Some(e)),
                }
            }
        }
    }
    (rows, None)
}

fn row_of(
    http: &dyn Http,
    opts: &Options,
    run: &Value,
    event: &str,
    attempt: u32,
    pulls: &mut BTreeMap<String, Option<(u64, String)>>,
) -> Result<Row, String> {
    let id = run.get("id").and_then(Value::as_u64).unwrap_or(0);
    let head_sha = str_of(run, "head_sha").to_string();
    let created_at = str_of(run, "created_at").to_string();
    let jobs_body = get(
        http,
        &format!(
            "/repos/{}/actions/runs/{id}/attempts/{attempt}/jobs?per_page={PER_PAGE}",
            opts.repo
        ),
    )?;
    let jobs = jobs_body
        .get("jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|j| job_of(http, opts.repo, j))
        .collect::<Result<Vec<Job>, String>>()?;
    let (pr, base_sha) = if event == "merge_group" {
        merge_queue_branch(str_of(run, "head_branch"))
            .map_or((None, None), |(n, sha)| (Some(n), Some(sha)))
    } else {
        let found = pulls
            .entry(head_sha.clone())
            .or_insert_with(|| pull_for(http, opts.repo, &head_sha))
            .clone();
        let base = found
            .as_ref()
            .zip(opts.clone)
            .and_then(|((_, branch), clone)| base_at(clone, branch, &created_at));
        (found.map(|(n, _)| n), base)
    };
    if let Some(clone) = opts.clone {
        let _ = git(clone, &["fetch", "-q", "origin", &head_sha]);
    }
    let conclusion = if jobs.iter().any(Job::failed) {
        "failure"
    } else {
        "success"
    };
    Ok(Row {
        repo: opts.repo.to_string(),
        run_id: id,
        attempt,
        event: event.to_string(),
        workflow: str_of(run, "name").to_string(),
        pr,
        head_sha,
        base_sha,
        created_at,
        conclusion: conclusion.to_string(),
        jobs,
    })
}
