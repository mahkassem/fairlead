//! `fairlead ci report` and the results file `ci run --results` writes: one
//! Markdown summary of a CI run, for the step summary and, when asked, one
//! pull request comment kept up to date.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use fairlead_core::config::LoadOptions;
use fairlead_core::plan::{Invocation, InvocationKind, Plan, Reason};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// How the comment is found again among the pull request's comments.
pub const MARKER: &str = "<!-- fairlead-report -->";
const SHOWN: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Results {
    pub version: u32,
    pub plan_id: String,
    pub passed: bool,
    pub seconds: f64,
    pub invocations: Vec<Ran>,
    /// Each failing test judged against the merge's plan, by `ci run --judge`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub judged: Vec<crate::ci_judge::Verdict>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ran {
    pub id: String,
    pub kind: InvocationKind,
    pub cwd: String,
    pub argv: Vec<String>,
    pub passed: bool,
    pub seconds: f64,
}

impl Ran {
    pub fn of(inv: &Invocation, passed: bool, started: Instant) -> Ran {
        Ran {
            id: inv.id.clone(),
            kind: inv.kind.clone(),
            cwd: inv.cwd.clone(),
            argv: inv.argv.clone(),
            passed,
            seconds: tenths(started.elapsed().as_secs_f64()),
        }
    }
}

fn tenths(seconds: f64) -> f64 {
    (seconds * 10.0).round() / 10.0
}

pub fn write_results(
    path: &Path,
    plan: &Plan,
    ran: Vec<Ran>,
    judged: &[crate::ci_judge::Verdict],
    started: Instant,
) -> Result<(), String> {
    let results = Results {
        version: 1,
        plan_id: plan.plan_id.clone(),
        passed: ran.iter().all(|r| r.passed) && ran.len() == plan.invocations.len(),
        seconds: tenths(started.elapsed().as_secs_f64()),
        invocations: ran,
        judged: judged.to_vec(),
    };
    let text = serde_json::to_string_pretty(&results).expect("results serialize") + "\n";
    std::fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

pub fn run(
    cwd: &Path,
    plan: &Path,
    results: Option<&Path>,
    receipt: Option<&Path>,
    comment: bool,
) -> Result<ExitCode, String> {
    let plan = crate::ci_cmd::read_plan(cwd, plan)?;
    let results: Option<Results> = match results {
        Some(p) => {
            let text = std::fs::read_to_string(cwd.join(p))
                .map_err(|e| format!("could not read {}: {e}", p.display()))?;
            Some(
                serde_json::from_str(&text)
                    .map_err(|e| format!("{} isn't a results file: {e}", p.display()))?,
            )
        }
        None => None,
    };
    if let Some(r) = &results {
        if r.plan_id != plan.plan_id {
            return Err(format!(
                "the results are for plan {}, not {}; pass the plan `ci run` ran",
                r.plan_id, plan.plan_id
            ));
        }
    }
    let receipt = match receipt {
        Some(p) => Some(
            std::fs::read_to_string(cwd.join(p))
                .map_err(|e| format!("could not read {}: {e}", p.display()))?,
        ),
        None => None,
    };
    let body = markdown(&plan, results.as_ref(), receipt.as_deref());
    match std::env::var_os("GITHUB_STEP_SUMMARY") {
        Some(target) => {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&target)
                .map_err(|e| format!("could not open $GITHUB_STEP_SUMMARY: {e}"))?;
            file.write_all(body.as_bytes())
                .map_err(|e| format!("could not write $GITHUB_STEP_SUMMARY: {e}"))?;
            println!("fairlead: the report is in the step summary");
        }
        None => print!("{body}"),
    }
    let ci = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
        .map(|l| l.config.ci)
        .unwrap_or_default();
    if comment || ci.comment {
        // A comment that can't be posted never fails the run; the summary has the report.
        match post(&body) {
            Ok(done) => println!("fairlead: {done}"),
            Err(e) => eprintln!("fairlead: the pull request comment wasn't posted: {e}"),
        }
    }
    let escaped = results.map_or(0, |r| r.judged.iter().filter(|v| v.fix.is_some()).count());
    if escaped > 0 && ci.escapes == fairlead_core::config::Escapes::Fail {
        eprintln!("fairlead: {escaped} failing test(s) escaped the merge's plan, and ci.escapes is \"fail\"");
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// The report: the headline, why the plan selected what it did, what ran,
/// what failed with the command to run it again, and the receipt.
pub fn markdown(plan: &Plan, results: Option<&Results>, receipt: Option<&str>) -> String {
    let mut out = format!("{MARKER}\n");
    let checks = plan.checks.len();
    let headline = match results {
        Some(r) => {
            let failed = r.invocations.iter().filter(|i| !i.passed).count();
            let outcome = if r.passed {
                "passed".to_string()
            } else {
                format!("{failed} of {} invocations failed", r.invocations.len())
            };
            format!("### Fairlead: {outcome} in {:.0} s\n\n", r.seconds)
        }
        None => "### Fairlead plan\n\n".into(),
    };
    out.push_str(&headline);
    let base = plan
        .base
        .as_deref()
        .map_or("no base".into(), |b| format!("`{}`", &b[..b.len().min(7)]));
    let selected = if plan.all {
        "everything (a change matched `plan.run_all` or reached every test)".to_string()
    } else {
        format!(
            "{} test file{} and {checks} check{}",
            plan.tests.len(),
            if plan.tests.len() == 1 { "" } else { "s" },
            if checks == 1 { "" } else { "s" }
        )
    };
    out.push_str(&format!(
        "Plan `{}` against {base}: {} changed file{}, selected {selected}.\n\n",
        plan.plan_id,
        plan.changed.len(),
        if plan.changed.len() == 1 { "" } else { "s" }
    ));
    let mut why: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &plan.tests {
        *why.entry(reason_class(&t.reason)).or_default() += 1;
    }
    if !why.is_empty() {
        out.push_str("| Why a test was selected | Tests |\n|---|---|\n");
        for (class, n) in &why {
            out.push_str(&format!("| {class} | {n} |\n"));
        }
        out.push('\n');
    }
    if !plan.unreached.is_empty() {
        let shown: Vec<String> = plan
            .unreached
            .iter()
            .take(SHOWN)
            .map(|u| format!("`{}` ({})", u.path, u.selected))
            .collect();
        let more = plan.unreached.len().saturating_sub(SHOWN);
        out.push_str(&format!(
            "Changed files no test reaches: {}{}.\n\n",
            shown.join(", "),
            if more > 0 {
                format!(", and {more} more")
            } else {
                String::new()
            }
        ));
    }
    if let Some(r) = results {
        out.push_str("| Ran | Result | Time |\n|---|---|---|\n");
        for i in &r.invocations {
            let result = if i.passed { "passed" } else { "**failed**" };
            out.push_str(&format!(
                "| `{}` in `{}` | {result} | {:.1} s |\n",
                i.id,
                dir(&i.cwd),
                i.seconds
            ));
        }
        out.push('\n');
        let failed: Vec<&Ran> = r.invocations.iter().filter(|i| !i.passed).collect();
        if !failed.is_empty() {
            out.push_str("To run a failed one again:\n\n```sh\n");
            for i in failed {
                out.push_str(&format!("(cd {} && {})\n", dir(&i.cwd), i.argv.join(" ")));
            }
            out.push_str("```\n\n");
        }
        let escaped: Vec<&crate::ci_judge::Verdict> =
            r.judged.iter().filter(|v| v.fix.is_some()).collect();
        if !escaped.is_empty() {
            out.push_str(&format!(
                "**{} escape{}**: failing tests the merge's plan left out.\n\n| Test | Rule that would have caught it |\n|---|---|\n",
                escaped.len(),
                if escaped.len() == 1 { "" } else { "s" }
            ));
            for v in &escaped {
                out.push_str(&format!(
                    "| `{}` | `{}` |\n",
                    v.test,
                    v.fix.as_deref().unwrap_or_default()
                ));
            }
            out.push('\n');
        }
        let planned = r.judged.len() - escaped.len();
        if planned > 0 {
            out.push_str(&format!(
                "{planned} failing test{} the merge's plan selected, so {} should have failed on the pull request too.\n\n",
                if planned == 1 { "" } else { "s" },
                if planned == 1 { "it" } else { "they" }
            ));
        }
    }
    if let Some(text) = receipt {
        out.push_str("<details><summary>Receipt</summary>\n\n```text\n");
        out.push_str(text.trim_end());
        out.push_str("\n```\n\n</details>\n");
    }
    out
}

fn dir(cwd: &str) -> &str {
    if cwd.is_empty() {
        "."
    } else {
        cwd
    }
}

fn reason_class(reason: &Reason) -> &'static str {
    match reason {
        Reason::RunAll { .. } => "a `plan.run_all` path changed",
        Reason::Changed => "the test itself changed",
        Reason::Import { .. } => "it imports a changed file",
        Reason::Owner { .. } => "an owner rule covers a changed file",
        Reason::Canary => "canary",
        Reason::Unreached { .. } => "a changed file nothing reaches",
        Reason::Paths { .. } | Reason::Modules { .. } | Reason::Always => "check rule",
    }
}

/// Posts the report as the pull request's comment, or updates the one
/// already there, found by its marker.
fn post(body: &str) -> Result<String, String> {
    let repo = std::env::var("GITHUB_REPOSITORY").map_err(|_| "GITHUB_REPOSITORY isn't set")?;
    let event = std::env::var("GITHUB_EVENT_PATH").map_err(|_| "GITHUB_EVENT_PATH isn't set")?;
    let event: Value = serde_json::from_str(
        &std::fs::read_to_string(&event).map_err(|e| format!("could not read {event}: {e}"))?,
    )
    .map_err(|e| format!("the event file isn't JSON: {e}"))?;
    let number = event["pull_request"]["number"]
        .as_u64()
        .ok_or("this run isn't for a pull request")?;
    let http = fairlead_replay::github::Curl::from_env();
    if http.token.is_none() {
        return Err("no GITHUB_TOKEN or GH_TOKEN".into());
    }
    let listed = http.send(
        "GET",
        &format!("/repos/{repo}/issues/{number}/comments?per_page=100"),
        &Value::Null,
    )?;
    if listed.status != 200 {
        return Err(format!(
            "listing comments answered {}: {}",
            listed.status, listed.message
        ));
    }
    let existing = listed
        .body
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["body"].as_str().is_some_and(|b| b.starts_with(MARKER)))
        .and_then(|c| c["id"].as_u64());
    let (method, path) = match existing {
        Some(id) => ("PATCH", format!("/repos/{repo}/issues/comments/{id}")),
        None => ("POST", format!("/repos/{repo}/issues/{number}/comments")),
    };
    let reply = http.send(method, &path, &json!({ "body": body }))?;
    if !(200..300).contains(&reply.status) {
        return Err(format!(
            "{method} answered {}: {}",
            reply.status, reply.message
        ));
    }
    Ok(match existing {
        Some(_) => format!("updated the report comment on #{number}"),
        None => format!("posted the report comment on #{number}"),
    })
}
