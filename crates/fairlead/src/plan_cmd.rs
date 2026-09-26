//! `fairlead plan` and `fairlead test --explain`: the changes since a base
//! commit, and the tests and checks they can affect.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use fairlead_core::config::{self, Config, LoadOptions};
use fairlead_core::plan::{json_schema, Change, Plan, Status};
use fairlead_lang::{build, Scan};
use fairlead_tests::{digest, explain::explain, git, plan, render, Input};

#[derive(Args, Clone)]
pub struct Changes {
    /// The branch or commit to compare with; its merge base with HEAD is used.
    /// Defaults to the remote's default branch.
    #[arg(long)]
    base: Option<String>,
    /// Plan for these paths instead of asking git.
    #[arg(long, num_args = 1..)]
    files: Vec<String>,
    /// Override a config value for this run.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    sets: Vec<String>,
}

pub struct Planned {
    pub plan: Plan,
    pub scan: Scan,
    pub config: Config,
}

fn explicit(root: &Path, cwd: &Path, files: &[String]) -> Vec<Change> {
    files
        .iter()
        .map(|f| {
            let abs = cwd.join(f);
            let rel = abs
                .strip_prefix(root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| f.replace('\\', "/"));
            let status = if abs.exists() {
                Status::Modified
            } else {
                Status::Deleted
            };
            Change {
                path: rel,
                status,
                from: None,
            }
        })
        .collect()
}

pub fn make(cwd: &Path, changes: &Changes) -> Result<Planned, String> {
    let loaded = config::load(cwd, &LoadOptions::from_process(changes.sets.clone()))
        .map_err(|e| e.to_string())?;
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
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let (changes, base) = if changes.files.is_empty() {
        let base = match &changes.base {
            Some(b) => b.clone(),
            None => git::default_base(&root)?,
        };
        let merge_base = git::merge_base(&root, &base).map_err(|e| {
            format!("{e}\nno merge base with {base}: in a shallow clone, fetch more history (fetch-depth: 0) or pass --files")
        })?;
        (git::changes(&root, &merge_base)?, Some(merge_base))
    } else {
        (explicit(&root, &cwd, &changes.files), None)
    };
    let mut scan = build(&root, &loaded.config)
        .map_err(|e| format!("could not read {}: {e}", root.display()))?;
    let clean = git::clean_tree_id(&root);
    let head = match &clean {
        Some(_) => git::head(&root).unwrap_or_else(|| "worktree".into()),
        None => "worktree".into(),
    };
    let tree_hash = clean.unwrap_or_else(|| digest::worktree_hash(&scan.tree));
    let input = Input {
        changes,
        base,
        head,
        config_digest: digest::config_digest(&loaded.config),
        tree_hash,
    };
    let plan = plan(&mut scan, &loaded.config, input)?;
    Ok(Planned {
        plan,
        scan,
        config: loaded.config,
    })
}

pub fn run_plan(
    cwd: &Path,
    changes: Changes,
    json: bool,
    out: Option<PathBuf>,
    schema: bool,
) -> ExitCode {
    if schema {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_schema()).expect("schema prints")
        );
        return ExitCode::SUCCESS;
    }
    let planned = match make(cwd, &changes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let text = serde_json::to_string_pretty(&planned.plan).expect("plan prints");
    if let Some(path) = out {
        if let Err(e) = std::fs::write(&path, format!("{text}\n")) {
            eprintln!("could not write {}: {e}", path.display());
            return ExitCode::from(2);
        }
    }
    if json {
        println!("{text}");
    } else {
        print!("{}", render::text(&planned.plan));
    }
    ExitCode::SUCCESS
}

pub fn run_explain(cwd: &Path, changes: Changes, target: &str) -> ExitCode {
    let planned = match make(cwd, &changes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let root = planned.scan.tree.root.clone();
    let abs = std::fs::canonicalize(cwd.join(target)).unwrap_or_else(|_| cwd.join(target));
    let rel =
        fairlead_lang::tree::relative(&root, &abs).unwrap_or_else(|| target.replace('\\', "/"));
    match explain(&planned.plan, &planned.scan, &planned.config, &rel) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
