//! `fairlead guard check`: the check stage over tracked files with the
//! ratchet, or with `--staged` the commit stage over what the next commit adds.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use clap::Subcommand;
use fairlead_core::config::{self, LoadOptions};
use fairlead_guard::baseline::{self, Counts};
use fairlead_guard::events::{Event, EventLog};
use fairlead_guard::{added, git, Finding, Guard, Source};

#[derive(Subcommand)]
pub enum GuardAction {
    /// Check tracked files against every rule, and ratcheted rules against the baseline.
    Check {
        /// Check only what the next commit adds: each staged file before and after.
        #[arg(long)]
        staged: bool,
        /// Print every finding, including those the baseline holds.
        #[arg(long)]
        list: bool,
        /// Record the ratcheted counts as they are now.
        #[arg(long, conflicts_with = "staged")]
        write_baseline: bool,
    },
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("guard: {message}");
    ExitCode::FAILURE
}

pub fn run(action: GuardAction, sets: Vec<String>, cwd: &Path) -> ExitCode {
    let loaded = match config::load(cwd, &LoadOptions::from_process(sets)) {
        Ok(loaded) => loaded,
        Err(e) => return fail(e),
    };
    if !loaded.problems.is_empty() {
        for p in &loaded.problems {
            eprintln!("{}: {}", p.key, p.message);
        }
        return fail("the config has problems; run `fairlead config check`");
    }
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    let guard = match Guard::new(&loaded.config.guard) {
        Ok(guard) => guard,
        Err(e) => return fail(e),
    };
    if guard.is_empty() {
        println!("guard: no rules configured");
        return ExitCode::SUCCESS;
    }
    let GuardAction::Check {
        staged,
        list,
        write_baseline,
    } = action;
    if staged {
        return check_staged(&root, &guard, &loaded.config.guard, list);
    }
    let baseline_path = root.join(&loaded.config.guard.baseline);
    check_tree(&root, &guard, &baseline_path, list, write_baseline)
}

fn check_tree(
    root: &Path,
    guard: &Guard,
    baseline_path: &Path,
    list: bool,
    write: bool,
) -> ExitCode {
    let files = match git::tracked(root) {
        Ok(files) => files,
        Err(e) => return fail(e),
    };
    let mut findings = Vec::new();
    for path in files.iter().filter(|p| guard.reads(p)) {
        // Deleted but not yet staged, or not text: nothing to read.
        let Ok(text) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        findings.extend(guard.lint(Source { path, text: &text }));
    }
    if list {
        for f in &findings {
            println!("{f}");
        }
    }
    let ratcheted = guard.ratcheted();
    let (held, zero): (Vec<&Finding>, Vec<&Finding>) =
        findings.iter().partition(|f| ratcheted.contains(f.rule));
    let now = baseline::tally(held.iter().copied());
    if write {
        return write_baseline(baseline_path, &now, held.len());
    }
    let was = match baseline::read(baseline_path) {
        Ok(counts) => only_rules(counts, &ratcheted),
        Err(e) => return fail(e),
    };
    let changes = baseline::changes(&now, &was);
    let (worse, fell): (Vec<_>, Vec<_>) = changes.into_iter().partition(|c| c.now > c.was);
    if !zero.is_empty() || !worse.is_empty() {
        eprintln!(
            "guard: {} finding(s) with no baseline, {} ratcheted file/rule pair(s) above the baseline",
            zero.len(),
            worse.len()
        );
        if !list {
            let over: BTreeSet<(&str, &str)> = worse
                .iter()
                .map(|w| (w.file.as_str(), w.rule.as_str()))
                .collect();
            let shown = held
                .iter()
                .filter(|f| over.contains(&(f.file.as_str(), f.rule)));
            for f in zero.iter().chain(shown) {
                eprintln!("{f}");
            }
        }
        for w in &worse {
            eprintln!(
                "  {} {}: {} allowed, {} found",
                w.file, w.rule, w.was, w.now
            );
        }
        eprintln!("Fix them, or record a ratcheted count on purpose with `fairlead guard check --write-baseline`.");
        return ExitCode::FAILURE;
    }
    println!(
        "guard: clean, {} finding(s) held at the baseline",
        held.len()
    );
    let mut by_rule = std::collections::BTreeMap::<&str, usize>::new();
    for f in &held {
        *by_rule.entry(f.rule).or_default() += 1;
    }
    for (rule, n) in by_rule {
        println!("  {rule}: {n}");
    }
    if !fell.is_empty() {
        println!(
            "guard: {} ratcheted count(s) fell; `fairlead guard check --write-baseline` lowers the baseline to match",
            fell.len()
        );
    }
    ExitCode::SUCCESS
}

/// A rule no longer ratcheted, or no longer configured, has nothing to hold.
fn only_rules(counts: Counts, rules: &BTreeSet<&'static str>) -> Counts {
    counts
        .into_iter()
        .map(|(file, by_rule)| {
            let kept = by_rule
                .into_iter()
                .filter(|(r, _)| rules.contains(r.as_str()))
                .collect();
            (file, kept)
        })
        .filter(
            |(_, by_rule): &(String, std::collections::BTreeMap<String, u64>)| !by_rule.is_empty(),
        )
        .collect()
}

fn write_baseline(path: &Path, counts: &Counts, total: usize) -> ExitCode {
    if let Err(e) = baseline::write(path, counts) {
        return fail(format!("{}: {e}", path.display()));
    }
    println!(
        "guard: baseline written to {}, {total} finding(s) across {} file(s)",
        path.display(),
        counts.len()
    );
    ExitCode::SUCCESS
}

fn check_staged(root: &Path, guard: &Guard, settings: &config::Guard, list: bool) -> ExitCode {
    let start = Instant::now();
    let staged = match git::staged(root) {
        Ok(staged) => staged,
        Err(e) => return fail(e),
    };
    let mut new = Vec::new();
    let mut read = 0;
    for file in staged.iter().filter(|s| guard.reads(&s.path)) {
        let Some(after) = git::in_index(root, &file.path) else {
            continue;
        };
        read += 1;
        // Both sides are linted under the new path, so a rename compares like an edit.
        let lint = |text: &str| {
            guard.lint(Source {
                path: &file.path,
                text,
            })
        };
        let now = lint(&after);
        if settings.findings == config::Findings::All {
            new.extend(now);
            continue;
        }
        let before = file
            .before
            .as_deref()
            .and_then(|b| git::at_head(root, b))
            .map(|text| lint(&text))
            .unwrap_or_default();
        new.extend(added::added(&before, &now));
    }
    fairlead_guard::sort(&mut new);
    let warn = settings.on_finding == config::OnFinding::Warn;
    let decision = match (new.is_empty(), warn) {
        (true, _) => "allow",
        (false, true) => "warn",
        (false, false) => "deny",
    };
    let mut event = Event::new("commit", decision, start.elapsed());
    event.files = Some(read);
    event.added = new.len();
    event.rules = new
        .iter()
        .map(|f| f.rule)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let log = match settings.events {
        config::Events::Local => EventLog::open(root),
        config::Events::Off => None,
    };
    if let Some(log) = log {
        // The log is a record, never a reason to block or fail a commit.
        let _ = log.append(&event);
    }
    if list {
        for f in &new {
            println!("{f}");
        }
    }
    let what = match settings.findings {
        config::Findings::Added => "the staged changes add",
        config::Findings::All => "the staged files have",
    };
    if new.is_empty() {
        println!("guard: {read} staged file(s), {what} no findings");
        return ExitCode::SUCCESS;
    }
    let then = if warn { ", committing anyway" } else { "" };
    eprintln!("guard: {what} {} finding(s){then}:", new.len());
    if !list {
        for f in &new {
            eprintln!("{f}");
        }
    }
    if warn {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
