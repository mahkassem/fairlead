//! The `fairlead ci settle` command: reads the plan and the results files,
//! checks the plan is this commit's and covers the event's whole change,
//! and fails when a step the plan holds has no green run.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Args, ValueEnum};
use fairlead_core::config::LoadOptions;
use fairlead_core::plan::{InvocationKind, Plan};
use serde_json::Value;

use crate::ci_report::Results;
use crate::ci_settle::{Proof, Record, Unsettled};
use crate::stage::Event;

/// The page that explains the check.
pub const PAGE: &str = "https://mahkassem.github.io/fairlead/docs/settle.html";
/// Test files named under an unsettled step before the rest are counted.
const SHOWN: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SettleFormat {
    Text,
    /// Also error annotations and the step summary.
    Github,
}

#[derive(Args)]
pub struct SettleArgs {
    /// The plan this run made with `ci plan`. Defaults to where `ci plan` writes it.
    #[arg(long, value_name = "PATH")]
    plan: Option<PathBuf>,
    /// A file `ci run --results` wrote, or a folder of them, such as the
    /// one the jobs' artifacts were downloaded into. Repeatable.
    #[arg(long, value_name = "PATH")]
    results: Vec<PathBuf>,
    /// The results of the jobs this one needs, as `${{ toJSON(needs) }}`
    /// gives them: a cancelled one means the run never finished.
    #[arg(long, value_name = "JSON")]
    needs: Option<String>,
    /// Also write the record here: what is settled, unsettled and waiting.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "text")]
    format: SettleFormat,
}

pub fn run(args: SettleArgs, cwd: &Path) -> Result<ExitCode, String> {
    let path = args.plan.unwrap_or_else(crate::ci_cmd::default_out);
    let plan = crate::ci_cmd::read_plan(cwd, &path)?;
    let root = crate::graph_cmd::repo_root(cwd);
    belongs(&plan, &root)?;
    covers(&plan, &root, cwd)?;
    if let Some(cancelled) = cancelled(args.needs.as_deref())? {
        eprintln!(
            "fairlead: not settled: the run was cancelled before {cancelled} finished, so nothing it left can count; a newer run that took its place has to collect it (`ci plan --since-green`)"
        );
        return Ok(ExitCode::FAILURE);
    }
    let mut results = Vec::new();
    for given in &args.results {
        read_into(&cwd.join(given), &mut results)?;
    }
    let attested = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
        .map_err(|e| e.to_string())?
        .config
        .ci
        .attested;
    let http = fairlead_replay::github::Curl::from_env();
    let jobs = crate::ci_settle_jobs::read(&http, &attested);
    let record = crate::ci_settle::settle(&plan, &results, &jobs);
    keep(&root, &record, args.out.as_deref().map(|p| cwd.join(p)))?;
    let mut body = text(&record, &plan);
    // What waits for the nightly run is only collected if that run is green.
    if record.waiting.iter().any(|w| w.due == "nightly") {
        if let Some(line) = crate::ci_settle_jobs::last_night(&http) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    print!("{body}");
    if args.format == SettleFormat::Github {
        github(&record, &body);
    }
    if record.passed() {
        return Ok(ExitCode::SUCCESS);
    }
    eprintln!("fairlead: {}", crate::report_cmd::MISTAKE);
    Ok(ExitCode::FAILURE)
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The plan has to be this commit's: its head and its tree.
fn belongs(plan: &Plan, root: &Path) -> Result<(), String> {
    if plan.head == "worktree" {
        return Err(
            "the plan was made on uncommitted changes; `ci settle` judges a commit, so plan a clean checkout"
                .into(),
        );
    }
    let head = fairlead_tests::git::head(root).ok_or("no commit here to settle")?;
    if head != plan.head {
        return Err(format!(
            "the plan is for commit {}, and this checkout is at {}; pass the plan this run made",
            short(&plan.head),
            short(&head)
        ));
    }
    let tree = crate::pr_receipt::tree_of(root, "HEAD");
    if tree.is_some_and(|t| t != plan.tree_hash) {
        return Err(format!(
            "the plan's tree is not commit {}'s; pass the plan this run made",
            short(&head)
        ));
    }
    Ok(())
}

/// The base commit the event's change starts from, where the event has one.
fn event_base(event: &Event) -> Option<String> {
    let payload = event.payload.as_ref()?;
    let sha = match event.name.as_str() {
        "pull_request" | "pull_request_target" => &payload["pull_request"]["base"]["sha"],
        "merge_group" => &payload["merge_group"]["base_sha"],
        _ => return None,
    };
    sha.as_str().map(String::from)
}

/// Is `older` an ancestor of `newer`: `None` when git can't tell.
fn ancestor(root: &Path, older: &str, newer: &str) -> Option<bool> {
    let status = Command::new("git")
        .current_dir(root)
        .args(["merge-base", "--is-ancestor", older, newer])
        .output()
        .ok()?
        .status;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// A plan that starts too late, or for too early a stage, would settle
/// green with part of the change never looked at.
fn covers(plan: &Plan, root: &Path, cwd: &Path) -> Result<(), String> {
    let event = Event::from_env();
    // A plan of everything covers the change wherever it starts.
    if !plan.all {
        let Some(base) = plan.base.as_deref() else {
            return Err(
                "the plan names no base commit (it was made with --files), so it can't say what the commit reaches; plan with --base"
                    .into(),
            );
        };
        if let Some(from) = event.as_ref().and_then(event_base) {
            starts_behind(root, base, &from)?;
        }
    }
    let (Some(stage), Some(event)) = (plan.stage, event) else {
        return Ok(());
    };
    let stages = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
        .map_err(|e| e.to_string())?
        .config
        .stages_or_default();
    let (due, why) = crate::stage::from_event(&event, &stages);
    if stage < due {
        return Err(format!(
            "the plan is for the {stage} stage and this run is the {due} stage ({why}): it leaves out what the {due} stage has to run; plan again without --stage"
        ));
    }
    Ok(())
}

/// The plan's base has to be the change's base or behind it.
fn starts_behind(root: &Path, base: &str, from: &str) -> Result<(), String> {
    match ancestor(root, base, from) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "the plan starts at {}, which is not behind the change's base {}: it leaves part of the change out; plan with --base {from}",
            short(base),
            short(from)
        )),
        None => Err(format!(
            "could not compare the plan's base {} with the change's base {}; fetch the full history (fetch-depth: 0)",
            short(base),
            short(from)
        )),
    }
}

/// The needed jobs GitHub cancelled, named, if any.
fn cancelled(needs: Option<&str>) -> Result<Option<String>, String> {
    let Some(text) = needs else {
        return Ok(None);
    };
    let jobs: BTreeMap<String, Value> = serde_json::from_str(text)
        .map_err(|e| format!("--needs isn't the JSON `toJSON(needs)` gives: {e}"))?;
    let names: Vec<&str> = (jobs.iter())
        .filter(|(_, job)| job["result"].as_str() == Some("cancelled"))
        .map(|(name, _)| name.as_str())
        .collect();
    Ok((!names.is_empty()).then(|| names.join(", ")))
}

/// Reads a results file, or every one under a folder; other JSON in a folder is skipped.
fn read_into(path: &Path, into: &mut Vec<Results>) -> Result<(), String> {
    if !path.is_dir() {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let read = serde_json::from_str(&text)
            .map_err(|e| format!("{} isn't a results file: {e}", path.display()))?;
        into.push(read);
        return Ok(());
    }
    let entries =
        std::fs::read_dir(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for inner in paths {
        if inner.is_dir() {
            read_into(&inner, into)?;
        } else if inner.extension().is_some_and(|x| x == "json") {
            let read = std::fs::read_to_string(&inner)
                .ok()
                .and_then(|t| serde_json::from_str::<Results>(&t).ok());
            into.extend(read);
        }
    }
    Ok(())
}

/// Writes the record where asked, and keeps it with the checkout's other
/// fairlead records, newest twenty.
fn keep(root: &Path, record: &Record, out: Option<PathBuf>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(record).expect("the record serializes") + "\n";
    if let Some(path) = out {
        std::fs::write(&path, &text)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    let Some(dir) = fairlead_guard::git::common_dir(root).map(|c| c.join("fairlead").join("debts"))
    else {
        return Ok(());
    };
    // Keeping a copy is a convenience; a read-only checkout still gets its verdict.
    if std::fs::create_dir_all(&dir).is_ok()
        && std::fs::write(dir.join(format!("{}.json", record.head)), &text).is_ok()
    {
        crate::ci_ledger::prune(&dir);
    }
    Ok(())
}

fn count(n: usize, one: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    }
}

fn unsettled_lines(u: &Unsettled) -> String {
    let shard = u.shard.map_or(String::new(), |k| format!(" (shard {k})"));
    let what = match u.kind {
        InvocationKind::Check => "check".to_string(),
        InvocationKind::Runner => count(u.files.len(), "test file"),
    };
    let mut out = format!("  {}{shard}: {}; {what}\n", u.id, u.why);
    for file in u.files.iter().take(SHOWN) {
        out.push_str(&format!("    {file}\n"));
    }
    if u.files.len() > SHOWN {
        out.push_str(&format!("    and {} more\n", u.files.len() - SHOWN));
    }
    out
}

/// What the check found, as it prints.
pub fn text(record: &Record, plan: &Plan) -> String {
    let head = short(&record.head);
    let files: usize = record.settled.iter().map(|s| s.files).sum();
    let mut out = String::new();
    match (record.unsettled.len(), record.settled.len()) {
        (0, 0) => out.push_str(&format!(
            "fairlead: settled: the plan for {head} holds nothing to run at this stage\n"
        )),
        (0, n) => out.push_str(&format!(
            "fairlead: settled: {} in {}, each green on {head}\n",
            count(files, "test file"),
            count(n, "step")
        )),
        (n, _) => out.push_str(&format!(
            "fairlead: not settled: {} on {head} with no green run\n",
            count(n, "step")
        )),
    }
    for u in &record.unsettled {
        out.push_str(&unsettled_lines(u));
    }
    for s in &record.settled {
        let by = match &s.proof {
            Proof::Results => "proven by results".to_string(),
            Proof::Job { name } => format!(
                "attested by job{} {name}, not proven by results",
                if name.contains(", ") { "s" } else { "" }
            ),
        };
        let what = match s.files {
            0 => String::new(),
            n => format!("{}, ", count(n, "test file")),
        };
        out.push_str(&format!("  {}: {what}{by}\n", s.id));
    }
    if !record.waiting.is_empty() {
        out.push_str("fairlead: waiting, not due at this stage:\n");
        for w in &record.waiting {
            let when = match w.due.as_str() {
                "nightly" => "due at the nightly run",
                _ => "due after the merge, on the push run",
            };
            let what = match w.files {
                0 => String::new(),
                n => format!("{}, ", count(n, "test file")),
            };
            out.push_str(&format!("  {}: {what}{when}\n", w.id));
        }
    }
    if !record.passed() {
        let stage = plan
            .stage
            .map_or(String::new(), |s| format!(" at the {s} stage"));
        out.push_str(&format!(
            "fairlead: every step the plan holds{stage} needs a green run on this commit; run the job that owns the step, with `ci run --results`. {PAGE}\n"
        ));
    }
    out
}

/// The annotations and the step summary GitHub shows on the run.
fn github(record: &Record, body: &str) {
    for u in &record.unsettled {
        println!(
            "::error title=fairlead ci settle::{} is not settled: {}",
            u.id, u.why
        );
    }
    let Some(target) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
        return;
    };
    let title = match record.passed() {
        true => "Settled",
        false => "Not settled",
    };
    let summary = format!("### fairlead ci settle: {title}\n\n```text\n{body}```\n");
    // The summary is a courtesy; the exit code is the verdict.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(target)
    {
        let _ = file.write_all(summary.as_bytes());
    }
}
