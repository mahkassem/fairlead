//! `fairlead skills`: what Fairlead does with the routed skills beyond the
//! brief. `sync` writes each one in the format each target agent loads.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::config::{Config, LoadOptions};

use crate::knowledge::sync::{self, Plan, State};

#[derive(clap::Subcommand)]
pub enum SkillsAction {
    /// Write each routed skill for each agent in `skills.targets`: `.claude/skills`, `.agents/skills` and `.cursor/rules`.
    Sync {
        /// Write nothing; exit 1 when a file is missing, stale or orphaned.
        #[arg(long)]
        check: bool,
        /// Remove every file sync wrote, found by its header, and nothing else.
        #[arg(long, conflicts_with = "check")]
        clean: bool,
    },
    /// Score skill routing on git history: a commit that changes code and edits a routed SKILL.md needed that skill.
    Eval {
        /// Every first-parent commit since this date, as `git log --since` reads it.
        #[arg(long, value_name = "DATE", conflicts_with = "limit")]
        since: Option<String>,
        /// How many first-parent commits from HEAD to read (500).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
        /// Print the scores as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Routing's hit rate from the event log: per skill and lesson, how often it was offered, used, used without an offer, and offered but not used.
    Report(crate::skills_report_cmd::ReportArgs),
}

fn config_at(cwd: &Path) -> Result<(PathBuf, Config), String> {
    let loaded = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
        .map_err(|e| e.to_string())?;
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    Ok((root, loaded.config))
}

pub fn run(action: SkillsAction, cwd: &Path) -> ExitCode {
    if let SkillsAction::Report(args) = action {
        return crate::skills_report_cmd::run(
            crate::skills_report_cmd::SkillsAction::Report(args),
            cwd,
        );
    }
    if let SkillsAction::Eval { since, limit, json } = action {
        return crate::skills_eval_cmd::run(
            crate::skills_eval_cmd::SkillsAction::Eval { since, limit, json },
            cwd,
        );
    }
    let (root, config) = match config_at(cwd) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("skills: {e}");
            return ExitCode::from(2);
        }
    };
    match action {
        SkillsAction::Sync { clean: true, .. } => clean(&root),
        SkillsAction::Sync { check: true, .. } => check(&root, &config),
        SkillsAction::Sync { .. } => write(&root, &config),
        SkillsAction::Eval { .. } | SkillsAction::Report(_) => unreachable!("handled above"),
    }
}

/// The plan for the tree as it is, resolving module routes to their roots
/// only when a Cursor rule needs globs for them.
pub fn plan(root: &Path, config: &Config) -> Plan {
    let skills = &config.skills;
    let wants_modules = skills.targets.items().iter().any(|t| t == "cursor")
        && skills.routes.items().iter().any(|r| !r.modules.is_empty());
    let modules = if wants_modules {
        let tree = fairlead_lang::tree::Tree::scan(root);
        let packages = fairlead_lang::workspace::discover(&tree);
        fairlead_tests::modules::Modules::discover(&tree, &packages, &config.modules)
            .unwrap_or_default()
    } else {
        fairlead_tests::modules::Modules::default()
    };
    let roots = |name: &str| -> Vec<String> {
        modules
            .all()
            .iter()
            .filter(|m| m.name == name)
            .map(|m| m.root.clone())
            .collect()
    };
    sync::plan(root, skills, &roots)
}

/// Removes a generated file, and its skill directory once nothing else is in it.
fn remove(root: &Path, path: &str) -> Result<(), String> {
    let full = root.join(path);
    std::fs::remove_file(&full).map_err(|e| format!("can't remove {path}: {e}"))?;
    if path.ends_with("/SKILL.md") {
        if let Some(dir) = full.parent() {
            let _ = std::fs::remove_dir(dir);
        }
    }
    Ok(())
}

fn write(root: &Path, config: &Config) -> ExitCode {
    let plan = plan(root, config);
    if !plan.problems.is_empty() {
        for p in &plan.problems {
            eprintln!("{p}");
        }
        eprintln!("skills sync: nothing written; fix the routes above first");
        return ExitCode::FAILURE;
    }
    let mut failed = false;
    for (t, state) in &plan.files {
        let full = root.join(&t.path);
        let done = match state {
            State::Fresh => Ok(format!("unchanged {}", t.path)),
            State::Conflict => {
                failed = true;
                Ok(sync::conflict(&t.path))
            }
            State::Missing | State::Stale => full
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&full, &t.text))
                .map(|()| format!("wrote {} (from {})", t.path, t.source))
                .map_err(|e| format!("can't write {}: {e}", t.path)),
        };
        match done {
            Ok(line) => println!("{line}"),
            Err(e) => {
                failed = true;
                eprintln!("{e}");
            }
        }
    }
    for o in &plan.orphans {
        match remove(root, o) {
            Ok(()) => println!("removed {o}"),
            Err(e) => {
                failed = true;
                eprintln!("{e}");
            }
        }
    }
    for n in &plan.not_copied {
        println!("not copied: {n} (sync copies SKILL.md only)");
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn check(root: &Path, config: &Config) -> ExitCode {
    let plan = plan(root, config);
    let drift = plan.drift();
    if drift.is_empty() {
        println!("skills in sync: {} file(s)", plan.files.len());
        return ExitCode::SUCCESS;
    }
    for line in &drift {
        println!("{line}");
    }
    println!("{} problem(s); run `fairlead skills sync`", drift.len());
    ExitCode::FAILURE
}

fn clean(root: &Path) -> ExitCode {
    let found = sync::generated(root);
    if found.is_empty() {
        println!("nothing to clean: no file carries the generated header");
        return ExitCode::SUCCESS;
    }
    let mut failed = false;
    for path in &found {
        match remove(root, path) {
            Ok(()) => println!("removed {path}"),
            Err(e) => {
                failed = true;
                eprintln!("{e}");
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// A doctor line when the synced skills have drifted, from the same plan `--check` reads.
pub fn doctor_line(root: &Path, config: &Config) -> Option<String> {
    if config.skills.routes.items().is_empty() && sync::generated(root).is_empty() {
        return None;
    }
    let drift = plan(root, config).drift();
    if drift.is_empty() {
        return None;
    }
    Some(format!(
        "skills sync: {} problem(s), such as `{}`; run `fairlead skills sync --check`\n",
        drift.len(),
        drift[0]
    ))
}
