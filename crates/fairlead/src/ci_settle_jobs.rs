//! What the CI run's own jobs concluded, for the steps the config says a job
//! outside `fairlead ci run` covers, and whether the last nightly run was
//! green, for what waits on it.

use std::collections::BTreeMap;

use fairlead_replay::github::Http;
use serde_json::Value;

use crate::ci_settle::Job;

fn var(name: &str) -> Result<String, String> {
    (std::env::var(name).ok().filter(|v| !v.is_empty()))
        .ok_or_else(|| format!("{name} isn't set, so this isn't a GitHub Actions run"))
}

fn get(http: &dyn Http, path: &str) -> Result<Value, String> {
    let reply = http.get_json(path)?;
    match reply.status {
        200 => Ok(reply.body),
        status => Err(format!("the GitHub API answered {status} for {path}")),
    }
}

/// What the jobs called `name` concluded: every one of them, a matrix's
/// legs included, has to have succeeded, and none at all is `absent`.
pub fn conclusion(jobs: &Value, name: &str) -> String {
    let leg = format!("{name} (");
    let named: Vec<&str> = (jobs["jobs"].as_array().into_iter().flatten())
        .filter(|j| (j["name"].as_str()).is_some_and(|n| n == name || n.starts_with(&leg)))
        .map(|j| j["conclusion"].as_str().unwrap_or("still running"))
        .collect();
    match named.iter().find(|c| **c != "success") {
        Some(other) => (*other).to_string(),
        None if named.is_empty() => "absent from this run".to_string(),
        None => "success".to_string(),
    }
}

/// The run's jobs, at their newest attempt.
fn listing(http: &dyn Http) -> Result<Value, String> {
    let repo = var("GITHUB_REPOSITORY")?;
    let run = var("GITHUB_RUN_ID")?;
    get(
        http,
        &format!("/repos/{repo}/actions/runs/{run}/jobs?per_page=100&filter=latest"),
    )
}

/// Each attested step's jobs with what they concluded in this run. A
/// listing that can't be read settles nothing, and says why.
pub fn read(http: &dyn Http, wanted: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<Job>> {
    if wanted.is_empty() {
        return BTreeMap::new();
    }
    let jobs = listing(http);
    let judge = |name: &String| Job {
        name: name.clone(),
        conclusion: match &jobs {
            Ok(jobs) => conclusion(jobs, name),
            Err(e) => format!("unread: {e}"),
        },
    };
    (wanted.iter())
        .map(|(step, names)| (step.clone(), names.iter().map(judge).collect()))
        .collect()
}

/// One line on this workflow's last finished nightly run, for a check that
/// leaves steps waiting on it; none where the API can't say.
pub fn last_night(http: &dyn Http) -> Option<String> {
    let repo = var("GITHUB_REPOSITORY").ok()?;
    let run = var("GITHUB_RUN_ID").ok()?;
    let current = get(http, &format!("/repos/{repo}/actions/runs/{run}")).ok()?;
    let workflow = current["workflow_id"].as_u64()?;
    let listed = get(
        http,
        &format!("/repos/{repo}/actions/workflows/{workflow}/runs?event=schedule&per_page=10"),
    )
    .ok()?;
    let newest = (listed["workflow_runs"].as_array().into_iter().flatten())
        .find(|r| r["status"].as_str() == Some("completed"));
    let Some(night) = newest else {
        return Some(
            "fairlead: no nightly run of this workflow has finished yet, so nothing has collected what waits for one"
                .to_string(),
        );
    };
    let day = night["created_at"]
        .as_str()
        .and_then(|t| t.get(..10))
        .unwrap_or("?");
    let sha = night["head_sha"]
        .as_str()
        .and_then(|s| s.get(..7))
        .unwrap_or("?");
    Some(match night["conclusion"].as_str() {
        Some("success") => format!("fairlead: the last nightly run was green, on {day} at {sha}"),
        other => format!(
            "fairlead: the last nightly run, on {day} at {sha}, ended {}: what waits for the nightly run is not collected until one is green",
            other.unwrap_or("unknown")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_job_of_the_name_has_to_succeed_and_none_is_absent() {
        let jobs = json!({"jobs": [
            {"name": "api", "conclusion": "success"},
            {"name": "browser (1/2)", "conclusion": "success"},
            {"name": "browser (2/2)", "conclusion": "failure"},
            {"name": "apidocs", "conclusion": "failure"},
            {"name": "slow", "conclusion": null},
            {"name": "gated", "conclusion": "skipped"},
        ]});
        assert_eq!(conclusion(&jobs, "api"), "success");
        assert_eq!(conclusion(&jobs, "browser"), "failure");
        assert_eq!(conclusion(&jobs, "slow"), "still running");
        assert_eq!(conclusion(&jobs, "gated"), "skipped");
        assert_eq!(conclusion(&jobs, "e2e"), "absent from this run");
    }
}
