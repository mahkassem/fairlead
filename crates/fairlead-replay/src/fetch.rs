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
use crate::window::add_days;

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
    /// Only runs of these workflows, by file name or id; every workflow when empty.
    pub workflows: Vec<String>,
    /// The last day to list; the listing runs to the present without one.
    pub until: Option<&'a str>,
}

/// Why a fetch stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// Every run in the window is recorded.
    Complete,
    /// `limit` new attempts were recorded; more remain.
    Limit,
    /// The API refused or failed; the rows so far are still good.
    Error(String),
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

/// Like `get`, but a resource GitHub no longer has (404, 410) reads as `null`.
fn get_gone_ok(http: &dyn Http, path: &str) -> Result<Value, String> {
    match http.get_json(path)? {
        (404 | 410, _) => Ok(Value::Null),
        _ => get(http, path),
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

fn number_and_base(pull: &Value) -> Option<(u64, String)> {
    Some((
        pull.get("number")?.as_u64()?,
        pull.get("base")?.get("ref")?.as_str()?.to_string(),
    ))
}

/// The pull request a run's head belongs to, and its base branch: from the
/// run, from the commit's pulls, or, for a fork, which neither names, from
/// the pulls whose head is the fork's branch. A refusal (rate limit) is an
/// error, so the row isn't written without it.
fn pull_for(http: &dyn Http, repo: &str, run: &Value) -> Result<Option<(u64, String)>, String> {
    let sha = str_of(run, "head_sha");
    if let Some(found) = run
        .get("pull_requests")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(number_and_base)
    {
        return Ok(Some(found));
    }
    let pulls = get_gone_ok(http, &format!("/repos/{repo}/commits/{sha}/pulls"))?;
    if let Some(found) = pulls
        .as_array()
        .and_then(|a| a.first())
        .and_then(number_and_base)
    {
        return Ok(Some(found));
    }
    let owner = run
        .get("head_repository")
        .and_then(|r| r.get("owner"))
        .map_or("", |o| str_of(o, "login"));
    let branch = str_of(run, "head_branch");
    let plain = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
    };
    if !plain(owner) || !plain(branch) {
        return Ok(None);
    }
    let by_head = get_gone_ok(
        http,
        &format!("/repos/{repo}/pulls?head={owner}:{branch}&state=all&per_page=10"),
    )?;
    // A fork's branch name can be reused; a pull request opened after the run
    // can't be the one it ran for.
    let started = str_of(run, "created_at");
    let candidates: Vec<Value> = by_head
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| started.is_empty() || str_of(p, "created_at") <= started)
        .collect();
    let exact = candidates
        .iter()
        .find(|p| p.get("head").map_or("", |h| str_of(h, "sha")) == sha);
    Ok(exact.or(candidates.first()).and_then(number_and_base))
}

/// The repository's default branch, the base when no pull request is found.
fn default_branch(http: &dyn Http, repo: &str) -> Result<Option<String>, String> {
    let body = get_gone_ok(http, &format!("/repos/{repo}"))?;
    Ok(Some(str_of(&body, "default_branch").to_string()).filter(|b| !b.is_empty()))
}

/// Answers already fetched, so each costs one request per fetch.
#[derive(Default)]
struct Lookups {
    pulls: BTreeMap<String, Option<(u64, String)>>,
    default_branch: Option<Option<String>>,
}

/// The base branch's first-parent commit when the run started.
fn base_at(clone: &Path, branch: &str, created_at: &str) -> Option<String> {
    git(
        clone,
        &["fetch", "-q", "origin", "--end-of-options", branch],
    )?;
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
    let notes = get_gone_ok(
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

/// Runs of the named workflows, or of every workflow when none is named.
/// Listing per workflow spends no requests on the others.
fn runs(http: &dyn Http, opts: &Options, event: &str) -> Result<Vec<Value>, String> {
    if opts.workflows.is_empty() {
        return runs_at(
            http,
            opts,
            event,
            &format!("/repos/{}/actions/runs", opts.repo),
        );
    }
    let mut all = Vec::new();
    for workflow in &opts.workflows {
        if workflow.is_empty()
            || !workflow
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        {
            return Err(format!("`{workflow}` isn't a workflow file name or id"));
        }
        let at = format!("/repos/{}/actions/workflows/{workflow}/runs", opts.repo);
        all.extend(runs_at(http, opts, event, &at)?);
    }
    Ok(all)
}

/// One listing query returns at most 1,000 runs, so the window is listed a
/// week at a time.
fn runs_at(http: &dyn Http, opts: &Options, event: &str, at: &str) -> Result<Vec<Value>, String> {
    let mut all = Vec::new();
    let mut from = opts.since.to_string();
    loop {
        let to = add_days(&from, 6).ok_or_else(|| format!("`{from}` isn't a date"))?;
        let last = opts.until.is_some_and(|u| to.as_str() >= u);
        let created = match (last, opts.until) {
            (true, Some(until)) => format!("{from}..{until}"),
            _ => format!("{from}..{to}"),
        };
        for page in 1.. {
            let path = format!(
                "{at}?event={event}&status=completed&created={created}&per_page={PER_PAGE}&page={page}"
            );
            let body = get(http, &path)?;
            let batch = body
                .get("workflow_runs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let total = body.get("total_count").and_then(Value::as_u64).unwrap_or(0) as usize;
            let done = batch.len() < PER_PAGE || page * PER_PAGE >= total;
            all.extend(batch.into_iter().filter(wanted));
            if done {
                break;
            }
        }
        if last || opts.until.is_none() && to.as_str() >= today().as_str() {
            break;
        }
        from = add_days(&to, 1).expect("a date plus one day");
    }
    Ok(all)
}

/// A run worth a row: not a first attempt that was cancelled or skipped,
/// which ran nothing.
fn wanted(run: &Value) -> bool {
    let first = run.get("run_attempt").and_then(Value::as_u64).unwrap_or(1) == 1;
    let empty = matches!(str_of(run, "conclusion"), "cancelled" | "skipped");
    !(first && empty)
}

/// Today in UTC, from the system clock.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    add_days("1970-01-01", (secs / 86_400) as i64).expect("a valid epoch date")
}

/// New rows for the runs in the window; `seen` holds (run, attempt) pairs
/// already recorded. On an API error the rows gathered so far come back
/// with it, so a partial fetch is never lost.
pub fn fetch(http: &dyn Http, opts: &Options, seen: &BTreeSet<(u64, u32)>) -> (Vec<Row>, Stop) {
    let mut rows = Vec::new();
    let mut seen = seen.clone();
    let mut lookups = Lookups::default();
    for event in EVENTS {
        let listed = match runs(http, opts, event) {
            Ok(r) => r,
            Err(e) => return (rows, Stop::Error(e)),
        };
        for run in listed {
            let id = run.get("id").and_then(Value::as_u64).unwrap_or(0);
            let attempts = run.get("run_attempt").and_then(Value::as_u64).unwrap_or(1) as u32;
            for attempt in 1..=attempts {
                // A run can shift onto the next page while listing.
                if !seen.insert((id, attempt)) {
                    continue;
                }
                if opts.limit.is_some_and(|l| rows.len() >= l) {
                    return (rows, Stop::Limit);
                }
                match row_of(http, opts, &run, event, attempt, &mut lookups) {
                    Ok(row) => rows.push(row),
                    Err(e) => return (rows, Stop::Error(e)),
                }
            }
        }
    }
    (rows, Stop::Complete)
}

fn row_of(
    http: &dyn Http,
    opts: &Options,
    run: &Value,
    event: &str,
    attempt: u32,
    lookups: &mut Lookups,
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
        let found = match lookups.pulls.get(&head_sha) {
            Some(found) => found.clone(),
            None => {
                let found = pull_for(http, opts.repo, run)?;
                lookups.pulls.insert(head_sha.clone(), found.clone());
                found
            }
        };
        let branch = match &found {
            Some((_, branch)) => Some(branch.clone()),
            None if opts.clone.is_none() => None,
            None => match &lookups.default_branch {
                Some(cached) => cached.clone(),
                None => {
                    let branch = default_branch(http, opts.repo)?;
                    lookups.default_branch = Some(branch.clone());
                    branch
                }
            },
        };
        let base = branch
            .zip(opts.clone)
            .and_then(|(branch, clone)| base_at(clone, &branch, &created_at));
        (found.map(|(n, _)| n), base)
    };
    if let Some(clone) = opts.clone {
        let _ = git(
            clone,
            &["fetch", "-q", "origin", "--end-of-options", &head_sha],
        );
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
