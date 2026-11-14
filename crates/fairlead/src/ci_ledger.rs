//! `fairlead ci ledger`: is this red mine? Reads a commit's CI runs, the
//! base branch's recent runs of the same workflows and every workflow's
//! runs on the default branch through the GitHub API, and gives each
//! failure a verdict: own, inherited, flaky or infra. It writes the ledger
//! to `.git/fairlead/ledger/<head sha>.json`, which `next`, `resume`,
//! `brief` and `doctor` read without touching the network.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::config::{Config, LoadOptions};
use fairlead_replay::extract::Extractor;
use fairlead_replay::github::{Curl, Http};
use fairlead_replay::ledger::{self, Evidence, Ledger, Proposal, RunRef, Verdict};
use fairlead_replay::live;
use regex::Regex;
use serde_json::Value;

use crate::ledger_text;

/// Days a proposed quarantine entry lasts before someone looks again.
pub const QUARANTINE_DAYS: i64 = 14;

#[derive(clap::Args)]
pub struct LedgerArgs {
    /// The repository, as owner/name; `GITHUB_REPOSITORY` or the `origin` remote by default.
    #[arg(long, value_name = "OWNER/NAME")]
    repo: Option<String>,
    /// Judge this run; in Actions, the current run (`GITHUB_RUN_ID`).
    #[arg(long, value_name = "ID", conflicts_with = "sha")]
    run: Option<u64>,
    /// Judge the newest runs on this commit; HEAD by default.
    #[arg(long, value_name = "SHA")]
    sha: Option<String>,
    /// The branch to compare with; the pull request's base, else the default branch.
    #[arg(long, value_name = "BRANCH")]
    base: Option<String>,
    /// How many of the base branch's runs of each workflow to read.
    #[arg(long, value_name = "N", default_value_t = 10)]
    history: usize,
    /// How many runs of each workflow on the default branch to read for red
    /// streaks; 0 skips them.
    #[arg(long, value_name = "N", default_value_t = 20)]
    streaks: usize,
    /// Also write the ledger here.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
    /// Read this ledger instead of the GitHub API, such as one CI wrote.
    #[arg(long, value_name = "PATH")]
    input: Option<PathBuf>,
    /// Append the ledger to $GITHUB_STEP_SUMMARY as Markdown.
    #[arg(long)]
    summary: bool,
    /// Print the ledger as JSON.
    #[arg(long)]
    json: bool,
    /// Rerun the failing tests no history explains: on HEAD, then on the
    /// merge base in a temporary worktree, to tell flaky from inherited
    /// from a regression.
    #[arg(long)]
    rerun: bool,
    /// The time the reruns may take altogether, in seconds.
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 300,
        requires = "rerun"
    )]
    rerun_seconds: u64,
    /// The most test files rerun.
    #[arg(long, value_name = "N", default_value_t = 10, requires = "rerun")]
    rerun_tests: usize,
}

pub fn run(args: LedgerArgs, cwd: &Path) -> Result<ExitCode, String> {
    let root = crate::graph_cmd::repo_root(cwd);
    let config = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
        .map(|l| l.config)
        .unwrap_or_default();
    let mut made = match &args.input {
        Some(path) => read(&cwd.join(path))?,
        None => fetch(&args, &root, &config)?,
    };
    if args.rerun {
        for line in crate::ledger_rerun::rerun(cwd, &mut made, args.rerun_seconds, args.rerun_tests)
        {
            eprintln!("ledger: {line}");
        }
    }
    made.proposals = proposals(&root, &made);
    if args.input.is_none() || args.rerun {
        if let Some(path) = cache_path(&root, &made.head_sha) {
            write(&path, &made)?;
            prune(path.parent().expect("a cache file has a folder"));
        }
    }
    if let Some(out) = &args.out {
        write(&cwd.join(out), &made)?;
    }
    if args.summary {
        if let Some(target) = std::env::var_os("GITHUB_STEP_SUMMARY") {
            std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&target)
                .and_then(|mut f| f.write_all(ledger_text::markdown(&made).as_bytes()))
                .map_err(|e| format!("could not write $GITHUB_STEP_SUMMARY: {e}"))?;
        }
    }
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&made).expect("ledger serializes")
        );
    } else {
        print!("{}", ledger_text::text(&made));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn read(path: &Path) -> Result<Ledger, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} isn't a ledger: {e}", path.display()))
}

fn write(path: &Path, made: &Ledger) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(made).expect("ledger serializes") + "\n";
    std::fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Where the ledger for `sha` is kept: in the main checkout's git
/// directory, never the working tree.
pub fn cache_path(root: &Path, sha: &str) -> Option<PathBuf> {
    let plain = !sha.is_empty() && sha.chars().all(|c| c.is_ascii_hexdigit());
    plain.then(|| cache_dir(root).map(|d| d.join(format!("{sha}.json"))))?
}

/// The folder every cached ledger is in.
pub fn cache_dir(root: &Path) -> Option<PathBuf> {
    fairlead_guard::git::common_dir(root).map(|c| c.join("fairlead").join("ledger"))
}

/// Ledgers kept, newest first; older ones are removed.
const KEPT: usize = 20;

pub(crate) fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, path) in files.into_iter().skip(KEPT) {
        let _ = std::fs::remove_file(path);
    }
}

/// The token from the environment, else from `gh auth token`.
fn http() -> Result<Curl, String> {
    let http = Curl::from_env_or_gh();
    if http.token.is_none() {
        return Err("ledger: no GITHUB_TOKEN or GH_TOKEN, and `gh auth token` gave none".into());
    }
    Ok(http)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The extractors `[[replay.failures]]` names, or with none, one per
/// runner whose command names a known test tool, over every job.
pub fn extractors(config: &Config) -> Result<Vec<(Regex, Extractor)>, String> {
    let declared = config.replay.failures.items();
    if !declared.is_empty() {
        return declared
            .iter()
            .map(|f| {
                let job = Regex::new(f.job.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
                Ok((job, Extractor::named(&f.extractor, f.pattern.as_deref())?))
            })
            .collect();
    }
    let mut names: Vec<&str> = Vec::new();
    for runner in config.tests.runners.items() {
        let guess = runner.command.iter().find_map(|word| {
            let tool = word.rsplit('/').next().unwrap_or(word);
            Some(match tool {
                "vitest" => "vitest",
                "jest" => "jest",
                "bun" => "bun",
                "pytest" => "pytest",
                "go" => "go",
                "phpunit" => "phpunit",
                "pest" | "artisan" => "pest",
                "mvn" | "mvnw" => "maven",
                "gradle" | "gradlew" => "gradle",
                _ => return None,
            })
        });
        if let Some(name) = guess.filter(|n| !names.contains(n)) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .map(|n| {
            Ok((
                Regex::new("").expect("empty regex"),
                Extractor::named(n, None)?,
            ))
        })
        .collect()
}

/// The runs to judge, each with its attempt, and every run on their commit.
type Judged = (Vec<(Value, u32)>, Vec<Value>);

/// The runs to judge: the one named, the current Actions run, or the
/// newest of each workflow on the commit.
fn runs_to_judge(
    http: &dyn Http,
    repo: &str,
    args: &LedgerArgs,
    root: &Path,
) -> Result<Judged, String> {
    let id = args
        .run
        .or_else(|| env("GITHUB_RUN_ID").and_then(|v| v.parse().ok()));
    if let Some(id) = id {
        let run = live::run(http, repo, id)?;
        let attempt = env("GITHUB_RUN_ATTEMPT")
            .filter(|_| args.run.is_none())
            .and_then(|a| a.parse().ok())
            .unwrap_or_else(|| run["run_attempt"].as_u64().unwrap_or(1) as u32);
        let sha = run["head_sha"].as_str().unwrap_or("").to_string();
        let on_commit = live::runs_on(http, repo, &sha)?;
        return Ok((vec![(run, attempt)], on_commit));
    }
    let sha = match &args.sha {
        Some(s) => crate::ci_cmd::rev_parse(root, s).unwrap_or_else(|| s.clone()),
        None => crate::ci_cmd::rev_parse(root, "HEAD").ok_or("ledger: couldn't read HEAD")?,
    };
    let on_commit = live::runs_on(http, repo, &sha)?;
    let judged = on_commit
        .iter()
        .map(|r| (r.clone(), r["run_attempt"].as_u64().unwrap_or(1) as u32))
        .collect();
    Ok((judged, on_commit))
}

fn base_of(
    args: &LedgerArgs,
    run: &Value,
    default: &mut dyn FnMut() -> Result<String, String>,
) -> Result<String, String> {
    if let Some(b) = args.base.clone().or_else(|| env("GITHUB_BASE_REF")) {
        return Ok(b);
    }
    let from_pull = run["pull_requests"][0]["base"]["ref"]
        .as_str()
        .filter(|b| !b.is_empty());
    match from_pull {
        Some(b) => Ok(b.to_string()),
        None => default(),
    }
}

fn fetch(args: &LedgerArgs, root: &Path, config: &Config) -> Result<Ledger, String> {
    let http = http()?;
    let repo = args
        .repo
        .clone()
        .or_else(|| env("GITHUB_REPOSITORY"))
        .or_else(|| crate::tracker::origin_repo(root))
        .ok_or("ledger: name the repository with --repo; there's no GITHUB_REPOSITORY or GitHub `origin` remote")?;
    let extractors = extractors(config)?;
    let made_at = fairlead_guard::events::timestamp(std::time::SystemTime::now());
    let (judged, on_commit) = runs_to_judge(&http, &repo, args, root)?;
    let mut default_branch: Option<String> = None;
    let mut default = || -> Result<String, String> {
        if default_branch.is_none() {
            default_branch = Some(live::default_branch(&http, &repo)?);
        }
        Ok(default_branch.clone().expect("just set"))
    };
    let mut made = Ledger {
        version: ledger::VERSION,
        repo: repo.clone(),
        made_at: made_at.clone(),
        head_sha: judged
            .first()
            .and_then(|(r, _)| r["head_sha"].as_str())
            .map(str::to_string)
            .or_else(|| crate::ci_cmd::rev_parse(root, "HEAD"))
            .unwrap_or_default(),
        base_branch: String::new(),
        runs: Vec::new(),
        failures: Vec::new(),
        proposals: Vec::new(),
        default_branch: None,
        streaks: Vec::new(),
    };
    for (run, attempt) in &judged {
        let id = run["id"].as_u64().unwrap_or(0);
        let jobs = live::jobs(&http, &repo, id, *attempt, None)?;
        let row = live::row_of(&repo, run, *attempt, jobs);
        made.runs.push(RunRef {
            run_id: id,
            attempt: *attempt,
            workflow: row.workflow.clone(),
            event: row.event.clone(),
            conclusion: row.conclusion.clone(),
        });
        let failing: BTreeSet<String> = row
            .jobs
            .iter()
            .filter(|j| j.failed())
            .map(|j| j.name.clone())
            .collect();
        if failing.is_empty() {
            continue;
        }
        let base = base_of(args, run, &mut default)?;
        made.base_branch = base.clone();
        let same_tree = live::same_tree(&http, &repo, run, &on_commit)?;
        let workflow_id = run["workflow_id"].as_u64().unwrap_or(0);
        let mut history =
            live::base_runs(&http, &repo, workflow_id, &base, args.history, &failing)?;
        // On the base branch itself, only runs before this one are its history.
        history.retain(|r| r.run_id != id && r.created_at < row.created_at);
        let ev = Evidence {
            same_tree: &same_tree,
            base: &history,
            base_branch: &base,
            extractors: &extractors,
        };
        made.failures.extend(ledger::judge(&row, &ev));
    }
    if args.streaks > 0 {
        let branch = default()?;
        for (rows, whole) in live::branch_history(&http, &repo, &branch, args.streaks)? {
            made.streaks.extend(fairlead_replay::streaks::streak(
                &rows,
                whole,
                &made_at,
                &extractors,
            ));
        }
        made.default_branch = Some(branch);
    }
    if made.base_branch.is_empty() {
        made.base_branch = made.default_branch.clone().unwrap_or_default();
    }
    Ok(made)
}

/// The test file a flaky failure names, as the repository has it.
fn repo_path(root: &Path, file: &str, project: Option<&str>) -> Option<String> {
    let file = file.replace('\\', "/");
    if root.join(&file).is_file() {
        return Some(file);
    }
    let tree = fairlead_lang::tree::Tree::scan(root);
    let packages: Vec<(String, String)> = fairlead_lang::workspace::discover(&tree)
        .into_iter()
        .map(|p| (p.name, p.dir))
        .collect();
    let repo = fairlead_replay::attribute::Repo {
        files: &tree.files,
        packages: &packages,
    };
    let printed = fairlead_replay::extract::Printed {
        path: file,
        project: project.map(str::to_string),
        title: None,
    };
    match fairlead_replay::attribute::attribute(&printed, &repo, |f| {
        std::fs::read_to_string(root.join(f)).ok()
    }) {
        fairlead_replay::attribute::Attribution::File(f) => Some(f),
        fairlead_replay::attribute::Attribution::Unattributed(_) => None,
    }
}

/// A `[[replay.quarantine]]` entry for each flaky test, for a person to
/// confirm; the file as the log printed it when the checkout doesn't have it.
fn proposals(root: &Path, made: &Ledger) -> Vec<Proposal> {
    let today = made.made_at.get(..10).unwrap_or("");
    let until = fairlead_replay::window::add_days(today, QUARANTINE_DAYS).unwrap_or_default();
    let mut out: Vec<Proposal> = Vec::new();
    for f in made.failures.iter().filter(|f| f.verdict == Verdict::Flaky) {
        let Some(file) = &f.file else {
            continue;
        };
        let path = repo_path(root, file, f.project.as_deref()).unwrap_or_else(|| file.clone());
        let job = format!("^{}$", regex::escape(&f.job));
        if out.iter().any(|p| p.path == path && p.job == job) {
            continue;
        }
        out.push(Proposal {
            path,
            job,
            reason: format!("{} in run {} on {today}", f.why, f.run_id),
            until: until.clone(),
        });
    }
    out
}
