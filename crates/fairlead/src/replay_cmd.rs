//! `fairlead replay run`: re-plan recorded CI failures from a benchmark
//! dataset and report recall. `replay fetch`, which records them, needs the
//! GitHub API and runs in CI.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use fairlead_core::config;
use fairlead_replay::dataset;
use fairlead_replay::fetch::Stop;
use fairlead_replay::git::Worktree;
use fairlead_replay::report::{report, text};
use fairlead_replay::run::{replay, Replayer, Sources};
use fairlead_replay::window::Window;

#[derive(Subcommand)]
pub enum ReplayAction {
    /// Record a repository's completed pull request and merge queue runs.
    /// Needs GITHUB_TOKEN (or GH_TOKEN) and curl.
    Fetch {
        /// The repository, as owner/name.
        #[arg(long)]
        repo: String,
        /// The dataset to append to.
        #[arg(long, value_name = "PATH")]
        data: PathBuf,
        /// The first day to list, YYYY-MM-DD.
        #[arg(long, value_name = "YYYY-MM-DD")]
        since: String,
        /// A clone to fetch each head into and read base commits from.
        #[arg(long, value_name = "DIR")]
        clone: Option<PathBuf>,
        /// Stop after this many run attempts.
        #[arg(long)]
        limit: Option<usize>,
        /// Only runs of this workflow, by name; repeat for several.
        #[arg(long = "workflow", value_name = "NAME")]
        workflows: Vec<String>,
    },
    /// Re-plan every recorded failure in the window and report recall.
    Run {
        /// The dataset, one JSON row per run attempt.
        #[arg(long, value_name = "PATH")]
        data: PathBuf,
        /// A clone of the repository the dataset describes.
        #[arg(long, value_name = "DIR")]
        clone: PathBuf,
        /// The config to plan with, kept outside the clone.
        #[arg(long, value_name = "PATH")]
        config: PathBuf,
        /// The last day of the window; defaults to the newest recorded run.
        #[arg(long, value_name = "YYYY-MM-DD")]
        until: Option<String>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
        /// Also write the report as JSON to this file.
        #[arg(long, value_name = "PATH")]
        json_out: Option<PathBuf>,
        /// Fetch the recorded heads and bases the clone lacks first.
        #[arg(long)]
        fetch_missing: bool,
    },
}

pub fn run(action: ReplayAction) -> ExitCode {
    let result = match action {
        ReplayAction::Run {
            data,
            clone,
            config,
            until,
            json,
            json_out,
            fetch_missing,
        } => run_replay(
            &data,
            &clone,
            &config,
            until.as_deref(),
            Output {
                json,
                json_out,
                fetch_missing,
            },
        ),
        ReplayAction::Fetch {
            repo,
            data,
            since,
            clone,
            limit,
            workflows,
        } => run_fetch(&repo, &data, &since, clone.as_deref(), limit, workflows),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}

fn run_fetch(
    repo: &str,
    data: &Path,
    since: &str,
    clone: Option<&Path>,
    limit: Option<usize>,
    workflows: Vec<String>,
) -> Result<(), String> {
    let seen = dataset::read(data)?
        .iter()
        .map(|r| (r.run_id, r.attempt))
        .collect();
    let http = fairlead_replay::github::Curl::from_env();
    let opts = fairlead_replay::fetch::Options {
        repo,
        since,
        clone,
        limit,
        workflows,
        until: None,
    };
    let (rows, stop) = fairlead_replay::fetch::fetch(&http, &opts, &seen);
    let added = dataset::append(data, &rows)?;
    let at = data.display();
    match stop {
        Stop::Complete => println!("fetch complete: {added} new rows in {at}"),
        Stop::Limit => println!("fetch partial: {added} new rows in {at}; run again to continue"),
        Stop::Error(e) => {
            println!("fetch stopped: {added} new rows in {at}");
            return Err(e);
        }
    }
    Ok(())
}

struct Output {
    json: bool,
    json_out: Option<PathBuf>,
    fetch_missing: bool,
}

fn run_replay(
    data: &Path,
    clone: &Path,
    config_path: &Path,
    until: Option<&str>,
    output: Output,
) -> Result<(), String> {
    let loaded = config::load_file(config_path, &[]).map_err(|e| e.to_string())?;
    let rows = dataset::read(data)?;
    let newest = rows
        .iter()
        .map(|r| r.created_at.clone())
        .max()
        .ok_or("the dataset has no rows")?;
    let until = until.map(str::to_string).unwrap_or(newest);
    let window = Window::ending(&until, loaded.config.replay.window_days)
        .ok_or_else(|| format!("`{until}` isn't a date"))?;
    let repo = rows[0].repo.clone();
    // Not canonicalize: on Windows that gives a `\\?\` path git may refuse.
    let clone = std::path::absolute(clone).map_err(|e| format!("{}: {e}", clone.display()))?;
    let wt_path = clone.with_file_name(format!(
        "{}-fairlead-replay",
        clone
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    if output.fetch_missing {
        let shas: Vec<String> = rows
            .iter()
            .filter(|r| window.contains(&r.created_at))
            .flat_map(|r| std::iter::once(r.head_sha.clone()).chain(r.base_sha.clone()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let missing = fairlead_replay::git::fetch_missing(&clone, &shas);
        eprintln!("{} recorded commits, {missing} still missing", shas.len());
    }
    let start = rows
        .iter()
        .find(|r| fairlead_replay::git::has_commit(&clone, &r.head_sha))
        .map(|r| r.head_sha.clone())
        .unwrap_or_else(|| "HEAD".into());
    let replayer = Replayer {
        clone: &clone,
        worktree: Worktree::open(&clone, &wt_path, &start)?,
        config: &loaded.config,
        sources: Sources::new(&loaded.config)?,
    };
    let replayed = replay(&replayer, &rows, &window);
    let result = report(&repo, &window, loaded.config.replay.min_failures, &replayed);
    let pretty = serde_json::to_string_pretty(&result).expect("report prints");
    if let Some(path) = &output.json_out {
        std::fs::write(path, format!("{pretty}\n"))
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    if output.json {
        println!("{pretty}");
    } else {
        print!("{}", text(&result));
    }
    Ok(())
}
