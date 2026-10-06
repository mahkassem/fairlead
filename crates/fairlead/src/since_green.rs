//! `ci plan --since-green JOB`: at the merge stage, plan from the head of the
//! newest earlier run of this workflow on this branch where JOB passed, so a
//! job that runs once per batch covers every commit merged since it last
//! passed. Any doubt about that commit plans everything instead.

use std::path::Path;
use std::process::Command;

use fairlead_core::config::CiStage;
use fairlead_core::plan::Warning;
use fairlead_replay::github::{Curl, Http};
use serde_json::Value;

use crate::plan_cmd::Changes;

/// Earlier runs read, newest first: one page, GitHub's largest.
const RUNS: usize = 100;

/// The run a base came from.
#[derive(Debug, PartialEq, Eq)]
pub struct Green {
    pub sha: String,
    pub run_id: u64,
    pub run_number: u64,
    pub url: String,
}

/// Why there's no base to trust, as a plan warning.
#[derive(Debug, PartialEq, Eq)]
pub struct Miss {
    pub code: &'static str,
    pub why: String,
}

fn unavailable(why: String) -> Miss {
    Miss {
        code: "since-green-unavailable",
        why,
    }
}

/// Set `changes`' base from the last green run of `jobs`, or plan everything
/// and return the warning that says why. Only the merge stage reads it.
pub fn apply(
    cwd: &Path,
    jobs: &[String],
    stage: Option<CiStage>,
    changes: &mut Changes,
) -> Option<Warning> {
    if jobs.is_empty() {
        return None;
    }
    if stage != Some(CiStage::Merge) {
        let at = stage.map_or("without a stage".to_string(), |s| {
            format!("at the {s} stage")
        });
        println!("since-green: ignored {at}; it applies at the merge stage");
        return None;
    }
    let http = Curl::from_env();
    let found = match http.token {
        None => Err(unavailable(
            "no GITHUB_TOKEN or GH_TOKEN to read past runs".into(),
        )),
        Some(_) => search(&http, jobs).and_then(|g| in_history(cwd, g)),
    };
    let names = jobs.join(", ");
    match found {
        Ok(g) => {
            let short = &g.sha[..12.min(g.sha.len())];
            println!(
                "since-green: base {short} from run {} (#{}), the newest where {names} passed; this batch is {short}..HEAD ({})",
                g.run_id, g.run_number, g.url
            );
            changes.base = Some(g.sha);
            None
        }
        Err(miss) => {
            println!("since-green: planning everything: {}", miss.why);
            changes.everything = Some(format!("no green base for {names}"));
            Some(Warning {
                code: miss.code.into(),
                path: None,
                message: miss.why,
            })
        }
    }
}

fn env(name: &str) -> Result<String, Miss> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            unavailable(format!(
                "{name} isn't set, so this isn't a GitHub Actions run"
            ))
        })
}

fn get(http: &dyn Http, path: &str) -> Result<Value, Miss> {
    let reply = http
        .get_json(path)
        .map_err(|e| unavailable(format!("the GitHub API failed: {e}")))?;
    if reply.status == 200 {
        return Ok(reply.body);
    }
    let limited = reply.status == 429 || (reply.status == 403 && reply.remaining == Some(0));
    let what = if limited { ", rate-limited" } else { "" };
    Err(unavailable(format!(
        "the GitHub API answered {}{what} for {path}: {}",
        reply.status, reply.message
    )))
}

/// The head of the newest earlier push or merge-queue run of this workflow on
/// this branch at which every one of `jobs` had passed at least once since.
pub fn search(http: &dyn Http, jobs: &[String]) -> Result<Green, Miss> {
    let repo = env("GITHUB_REPOSITORY")?;
    let run_id = env("GITHUB_RUN_ID")?;
    let current = get(http, &format!("/repos/{repo}/actions/runs/{run_id}"))?;
    let (Some(workflow), Some(branch)) = (
        current["workflow_id"].as_u64(),
        current["head_branch"].as_str(),
    ) else {
        return Err(unavailable(format!(
            "run {run_id} has no workflow or branch in the API's answer"
        )));
    };
    let number = current["run_number"].as_u64().unwrap_or(u64::MAX);
    let listed = get(
        http,
        &format!(
            "/repos/{repo}/actions/workflows/{workflow}/runs?branch={}&per_page={RUNS}",
            encode(branch)
        ),
    )?;
    let mut waiting: Vec<&str> = jobs.iter().map(String::as_str).collect();
    let runs = listed["workflow_runs"].as_array().into_iter().flatten();
    for run in runs.take(RUNS) {
        if !proves(run, &run_id, number, branch) {
            continue;
        }
        let (Some(id), Some(sha)) = (run["id"].as_u64(), run["head_sha"].as_str()) else {
            continue;
        };
        let jobs = get(
            http,
            &format!("/repos/{repo}/actions/runs/{id}/jobs?per_page=100"),
        )?;
        waiting.retain(|job| !passed(&jobs, job));
        if waiting.is_empty() {
            return Ok(Green {
                sha: sha.to_string(),
                run_id: id,
                run_number: run["run_number"].as_u64().unwrap_or(0),
                url: run["html_url"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    Err(Miss {
        code: "since-green-not-found",
        why: format!(
            "no earlier push run on {branch} in the last {RUNS} has {} passing",
            waiting.join(", ")
        ),
    })
}

/// An earlier run that proves what merged: a push or merge queue on the same
/// branch. A pull request run tested a commit that never landed as such.
fn proves(run: &Value, current_id: &str, current_number: u64, branch: &str) -> bool {
    let event = run["event"].as_str().unwrap_or("");
    let earlier = run["run_number"]
        .as_u64()
        .is_some_and(|n| n < current_number)
        && run["id"].as_u64().map(|id| id.to_string()).as_deref() != Some(current_id);
    matches!(event, "push" | "merge_group")
        && run["head_branch"].as_str() == Some(branch)
        && earlier
}

fn passed(jobs: &Value, name: &str) -> bool {
    jobs["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|j| j["name"].as_str() == Some(name) && j["conclusion"].as_str() == Some("success"))
}

/// A query value with everything but unreserved characters escaped.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn git_ok(cwd: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The commit must be in this clone and an ancestor of HEAD: a base from
/// rewritten history would skip changes nothing tested.
fn in_history(cwd: &Path, green: Green) -> Result<Green, Miss> {
    let miss = |why: String| Miss {
        code: "since-green-not-in-history",
        why,
    };
    if green.sha.len() != 40 || !green.sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(unavailable(format!(
            "run {} has no commit sha in the API's answer",
            green.run_id
        )));
    }
    let commit = format!("{}^{{commit}}", green.sha);
    if !git_ok(cwd, &["cat-file", "-e", &commit]) {
        return Err(miss(format!(
            "{} from run {} isn't in this clone; fetch history (fetch-depth: 0)",
            green.sha, green.run_id
        )));
    }
    if !git_ok(cwd, &["merge-base", "--is-ancestor", &green.sha, "HEAD"]) {
        return Err(miss(format!(
            "{} from run {} isn't an ancestor of HEAD",
            green.sha, green.run_id
        )));
    }
    Ok(green)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_branch_is_escaped_in_the_query() {
        assert_eq!(encode("release/2026 x"), "release%2F2026%20x");
        assert_eq!(encode("main"), "main");
    }

    #[test]
    fn only_an_earlier_push_or_merge_queue_run_on_the_branch_proves_a_merge() {
        let run = |event: &str, branch: &str, id: u64, number: u64| json!({"event": event, "head_branch": branch, "id": id, "run_number": number});
        assert!(proves(&run("push", "main", 1, 1), "9", 5, "main"));
        assert!(proves(&run("merge_group", "main", 1, 1), "9", 5, "main"));
        assert!(!proves(&run("pull_request", "main", 1, 1), "9", 5, "main"));
        assert!(!proves(&run("push", "dev", 1, 1), "9", 5, "main"));
        assert!(!proves(&run("push", "main", 9, 1), "9", 5, "main"));
        assert!(!proves(&run("push", "main", 1, 6), "9", 5, "main"));
    }
}
