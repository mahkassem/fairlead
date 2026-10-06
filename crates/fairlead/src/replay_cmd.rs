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
use fairlead_replay::run::{replay_with, Progress, Replayer, Sources};
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
        /// Only runs of this workflow, by file name (such as ci.yml) or id;
        /// repeat for several.
        #[arg(long = "workflow", value_name = "NAME")]
        workflows: Vec<String>,
        /// Also record this event's runs: `push`, the default branch's push
        /// runs, where a failure the merge's plan left out is an escape, or
        /// `schedule`, its scheduled full runs, where a failure no push's plan
        /// reached since the last green one is. Repeat for several;
        /// pull_request and merge_group are always read.
        #[arg(long = "event", value_name = "EVENT")]
        events: Vec<String>,
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
        /// No progress lines on stderr.
        #[arg(long)]
        quiet: bool,
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
            quiet,
        } => run_replay(
            &data,
            &clone,
            &config,
            until.as_deref(),
            Output {
                json,
                json_out,
                fetch_missing,
                quiet,
            },
        ),
        ReplayAction::Fetch {
            repo,
            data,
            since,
            clone,
            limit,
            workflows,
            events,
        } => run_fetch(
            &repo,
            &data,
            &since,
            clone.as_deref(),
            limit,
            workflows,
            events,
        ),
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
    extra: Vec<String>,
) -> Result<(), String> {
    let mut events: Vec<String> = fairlead_replay::fetch::EVENTS.map(String::from).to_vec();
    for event in extra {
        if !events.contains(&event) {
            events.push(event);
        }
    }
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
        events,
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
    quiet: bool,
}

fn run_replay(
    data: &Path,
    clone: &Path,
    config_path: &Path,
    until: Option<&str>,
    output: Output,
) -> Result<(), String> {
    let loaded = config::load_file(config_path, &[]).map_err(|e| e.to_string())?;
    if !loaded.problems.is_empty() {
        let lines: Vec<String> = loaded
            .problems
            .iter()
            .map(|p| format!("  {}: {}", p.key, p.message))
            .collect();
        return Err(format!(
            "the config has problems (see `fairlead config check`):\n{}",
            lines.join("\n")
        ));
    }
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
        eprintln!(
            "fetching the recorded commits the clone lacks, of {}",
            shas.len()
        );
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
    let mut lines = ProgressLines::new(output.quiet);
    let replayed = replay_with(&replayer, &rows, &window, &mut |p| lines.after(p));
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

/// How often a progress line is printed: every this many runs, and at least
/// once a minute.
const EVERY_RUNS: usize = 25;
const EVERY_SECONDS: u64 = 60;
/// A run slower than this gets its own line, so a stall shows where it is.
const SLOW_SECONDS: f64 = 10.0;

/// Progress on stderr as plain lines, never redrawn in place, so a CI log
/// keeps the history. The report on stdout is untouched.
struct ProgressLines {
    quiet: bool,
    started: std::time::Instant,
    last: std::time::Instant,
}

impl ProgressLines {
    fn new(quiet: bool) -> ProgressLines {
        let now = std::time::Instant::now();
        ProgressLines {
            quiet,
            started: now,
            last: now,
        }
    }

    fn after(&mut self, p: &Progress) {
        if self.quiet {
            return;
        }
        if p.seconds > SLOW_SECONDS {
            eprintln!(
                "run {} took {} to plan and judge",
                p.run_id,
                duration(p.seconds)
            );
        }
        let due = p.planned.is_multiple_of(EVERY_RUNS)
            || p.planned == p.runs
            || self.last.elapsed().as_secs() >= EVERY_SECONDS;
        if due {
            self.last = std::time::Instant::now();
            eprintln!("{}", progress_line(p, self.started.elapsed().as_secs_f64()));
        }
    }
}

/// `412 of 806 runs planned · 93 failures judged · 3 misses · 10 min, about 10 min left`
fn progress_line(p: &Progress, elapsed: f64) -> String {
    let mut line = format!(
        "{} of {} runs planned · {} failures judged · {} misses · {}",
        p.planned,
        p.runs,
        p.judged,
        p.misses,
        duration(elapsed)
    );
    // Too few runs make a guess, not an estimate.
    if p.planned >= EVERY_RUNS && p.planned < p.runs {
        let left = elapsed / p.planned as f64 * (p.runs - p.planned) as f64;
        line.push_str(&format!(", about {} left", duration(left)));
    }
    line
}

fn duration(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{} s", seconds.round() as u64)
    } else {
        format!("{} min", (seconds / 60.0).round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(planned: usize, runs: usize) -> Progress {
        Progress {
            planned,
            runs,
            judged: 93,
            misses: 3,
            run_id: 1,
            seconds: 1.0,
        }
    }

    #[test]
    fn a_progress_line_counts_and_estimates_once_there_are_enough_runs() {
        assert_eq!(
            progress_line(&at(412, 806), 600.0),
            "412 of 806 runs planned · 93 failures judged · 3 misses · 10 min, about 10 min left"
        );
        assert_eq!(
            progress_line(&at(3, 806), 12.4),
            "3 of 806 runs planned · 93 failures judged · 3 misses · 12 s"
        );
        assert!(!progress_line(&at(806, 806), 900.0).contains("left"));
    }
}
