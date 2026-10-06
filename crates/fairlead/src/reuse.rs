//! Tree reuse: a pull request that passed its ready stage records the tree
//! it tested as a commit status, and a push to an environment branch whose
//! tree is that exact tree skips the ready stage's steps instead of running
//! them again. Anything short of an exact match runs them.

use fairlead_core::config::{CiStage, Config};
use fairlead_core::plan::{InvocationKind, Plan, Reused};
use fairlead_replay::github::Curl;
use serde_json::{json, Value};

use crate::stage::{from_of, Event};

pub const CONTEXT: &str = "fairlead/tree";

/// What the status says: the tree and the config's digest, which together
/// decide what a plan of that tree runs and how.
fn description(plan: &Plan) -> Option<String> {
    let digest = plan.config_digest.strip_prefix("sha256:")?;
    let tree = &plan.tree_hash;
    let clean = tree.len() == 40 && tree.bytes().all(|b| b.is_ascii_hexdigit());
    clean.then(|| format!("tree {tree} config {}", &digest[..16.min(digest.len())]))
}

fn repo() -> Result<String, String> {
    std::env::var("GITHUB_REPOSITORY").map_err(|_| "GITHUB_REPOSITORY isn't set".to_string())
}

/// After a ready plan passed every step: the status on the pull request's
/// head commit. Returns the line to print, or `None` when nothing applies.
pub fn record(plan: &Plan, config: &Config, ran_all: bool) -> Option<String> {
    let stages = config.stages.as_ref()?;
    if !stages.reuse || plan.stage != Some(CiStage::Ready) {
        return None;
    }
    if !ran_all {
        return Some("fairlead: not recording the tree: only part of the plan ran".into());
    }
    let Some(description) = description(plan) else {
        return Some("fairlead: not recording the tree: the working tree isn't a commit".into());
    };
    let event = Event::from_env()?;
    let head = event
        .payload
        .as_ref()
        .and_then(|p| p.pointer("/pull_request/head/sha"))
        .and_then(Value::as_str)?
        .to_string();
    let outcome = repo().and_then(|repo| {
        let http = Curl::from_env();
        if http.token.is_none() {
            return Err("no GITHUB_TOKEN or GH_TOKEN".into());
        }
        let body = json!({"state": "success", "context": CONTEXT, "description": description});
        let reply = http.send("POST", &format!("/repos/{repo}/statuses/{head}"), &body)?;
        match reply.status {
            200..=299 => Ok(()),
            403 => Err("403: give the job `statuses: write`".into()),
            s => Err(format!("{s}: {}", reply.message)),
        }
    });
    Some(match outcome {
        Ok(()) => format!("fairlead: recorded {CONTEXT} on {head} for reuse at merge"),
        Err(e) => {
            format!("fairlead: couldn't record {CONTEXT}, so a merge runs these steps again: {e}")
        }
    })
}

/// The pull request a pushed commit came from and its head commit's
/// `fairlead/tree` description, read from the API.
fn recorded(http: &Curl, repo: &str, sha: &str) -> Result<Option<(u64, String)>, String> {
    let pulls = http.send(
        "GET",
        &format!("/repos/{repo}/commits/{sha}/pulls"),
        &Value::Null,
    )?;
    if pulls.status != 200 {
        return Err(format!(
            "listing its pull requests answered {}",
            pulls.status
        ));
    }
    let Some(pr) = pulls
        .body
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p.get("merged_at").is_some_and(|m| !m.is_null()))
    else {
        return Ok(None);
    };
    let number = pr["number"].as_u64().unwrap_or(0);
    let Some(head) = pr.pointer("/head/sha").and_then(Value::as_str) else {
        return Ok(None);
    };
    let statuses = http.send(
        "GET",
        &format!("/repos/{repo}/commits/{head}/statuses?per_page=100"),
        &Value::Null,
    )?;
    if statuses.status != 200 {
        return Err(format!("reading its statuses answered {}", statuses.status));
    }
    // Newest first: the latest word on this context is the one that counts.
    let latest = statuses
        .body
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["context"] == CONTEXT);
    Ok(latest
        .filter(|s| s["state"] == "success")
        .and_then(|s| s["description"].as_str())
        .map(|d| (number, d.to_string())))
}

/// At merge: when the pull request recorded this exact tree and config,
/// drop the ready stage's steps from `plan` and say so. Returns the line to print.
pub fn apply(plan: &mut Plan, config: &Config, event: Option<&Event>) -> Option<String> {
    let stages = config.stages.as_ref()?;
    if !stages.reuse || plan.stage != Some(CiStage::Merge) {
        return None;
    }
    let event = event.filter(|e| e.name == "push")?;
    let sha = event
        .payload
        .as_ref()
        .and_then(|p| p.get("after"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| std::env::var("GITHUB_SHA").ok())?;
    let Some(want) = description(plan) else {
        return Some("reuse: no, the working tree isn't a commit".into());
    };
    let http = Curl::from_env();
    let found = match repo().and_then(|repo| recorded(&http, &repo, &sha)) {
        Ok(found) => found,
        Err(e) => return Some(format!("reuse: no, {e}; the ready steps run")),
    };
    let Some((number, have)) = found else {
        return Some("reuse: no pull request recorded a tree for this commit".into());
    };
    if have != want {
        return Some(format!(
            "reuse: no, #{number} passed another tree or config ({have}); the ready steps run"
        ));
    }
    let ready = |id: &str, kind: &InvocationKind| from_of(config, id, kind) <= CiStage::Ready;
    let mut steps: Vec<String> = Vec::new();
    for inv in plan.invocations.iter().filter(|i| ready(&i.id, &i.kind)) {
        if !steps.contains(&inv.id) {
            steps.push(inv.id.clone());
        }
    }
    plan.invocations.retain(|i| !ready(&i.id, &i.kind));
    plan.tests.retain(|t| {
        t.runner
            .as_deref()
            .is_none_or(|r| !ready(r, &InvocationKind::Runner))
    });
    plan.checks
        .retain(|c| !ready(&c.id, &InvocationKind::Check));
    let line = format!(
        "reuse: #{number} passed this tree; skipping {}",
        if steps.is_empty() {
            "nothing".into()
        } else {
            steps.join(", ")
        }
    );
    plan.plan_id = fairlead_tests::digest::with_stage(&plan.plan_id, "reused");
    plan.reused = Some(Reused {
        pull_request: number,
        steps,
    });
    Some(line)
}
