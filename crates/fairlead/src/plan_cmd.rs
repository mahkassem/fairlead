//! `fairlead plan` and `fairlead test --explain`: the changes since a base
//! commit, and the tests and checks they can affect.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use fairlead_core::config::{self, Config, LoadOptions};
use fairlead_core::plan::{json_schema, Change, Plan, Status};
use fairlead_lang::tree::Tree;
use fairlead_lang::{build, build_sharing, Scan};
use fairlead_tests::{digest, explain::explain, git, plan, render, Input};

#[derive(Args, Clone)]
pub struct Changes {
    /// The branch or commit to compare with; its merge base with HEAD is used.
    /// Defaults to the remote's default branch.
    #[arg(long)]
    pub base: Option<String>,
    /// Plan for these paths instead of asking git.
    #[arg(long, num_args = 1..)]
    pub files: Vec<String>,
    /// Override a config value for this run.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub sets: Vec<String>,
    /// In a workspace, only this repository.
    #[arg(long, value_name = "NAME")]
    pub repo: Option<String>,
    /// Plan everything whatever changed, for this reason; a full CI stage sets it.
    #[arg(skip)]
    pub everything: Option<String>,
    /// Run whole suites that wait for the full stage; `plan` and `done` set it from `--full`.
    #[arg(skip)]
    pub full_suites: bool,
    /// The CI stage the plan is for; `ci plan` sets it.
    #[arg(skip)]
    pub stage: Option<fairlead_core::config::CiStage>,
    /// Scan with the parse cache of the checkout here, not the scanned one's.
    #[arg(skip)]
    pub cache_from: Option<PathBuf>,
}

impl Changes {
    /// The changes a caller names itself, as `--base`, `--files` and `--set` would.
    pub fn new(base: Option<String>, files: Vec<String>, sets: Vec<String>) -> Changes {
        Changes {
            base,
            files,
            sets,
            repo: None,
            everything: None,
            full_suites: false,
            stage: None,
            cache_from: None,
        }
    }
}

/// The config a plan of `changes` reads, with its `--set` values, for a
/// caller that decides something before planning.
pub fn config_for(cwd: &Path, changes: &Changes) -> Result<Config, String> {
    config::load(cwd, &LoadOptions::from_process(changes.sets.clone()))
        .map(|l| l.config)
        .map_err(|e| e.to_string())
}

/// Said once when a plan falls back to HEAD for want of a base branch.
pub const NO_BASE: &str = "fairlead: no base branch found (`[pr] base`, the remote's default branch, `main` or `master`), so this plan covers only what changed since HEAD; pass --base to compare with a branch";

/// The branch a change is compared with when none is given: `[pr] base`,
/// its remote-tracking ref when fetched, else the remote's default branch,
/// else `main` or `master`. None when none of them exists.
pub fn default_base(root: &Path, config: Option<&Config>) -> Option<String> {
    if let Some(base) = config
        .and_then(|c| c.pr.as_ref())
        .and_then(|p| p.base.as_deref())
    {
        let remote = fairlead_guard::fetch::remote_for(root, Some(base));
        for rev in [remote.tracking.as_str(), base] {
            if crate::head::commit(root, rev).is_ok() {
                return Some(rev.to_string());
            }
        }
    }
    git::default_base(root).ok()
}

/// The commit a change is measured from: its merge base with `given`, else
/// with the default base, else HEAD.
pub fn merge_base_or_head(
    root: &Path,
    given: Option<&str>,
    config: Option<&Config>,
) -> Option<String> {
    let base = given
        .map(String::from)
        .or_else(|| default_base(root, config));
    match base {
        Some(base) => git::merge_base(root, &base).ok(),
        None => git::head(root),
    }
}

pub struct Planned {
    pub plan: Plan,
    pub scan: Scan,
    pub config: Config,
}

/// Explicit paths as changes: relative to `cwd`, `..` resolved, a directory
/// standing for every file under it, and a missing path counted as deleted.
fn explicit(root: &Path, cwd: &Path, tree: &Tree, files: &[String]) -> Result<Vec<Change>, String> {
    let base = fairlead_lang::tree::relative(root, cwd).unwrap_or_default();
    let mut changes = Vec::new();
    for f in files {
        let given = Path::new(f);
        let rel = if given.is_absolute() {
            let abs = std::fs::canonicalize(given).unwrap_or_else(|_| given.to_path_buf());
            fairlead_lang::tree::relative(root, &abs)
        } else {
            fairlead_lang::tree::normalize(&base, &f.replace('\\', "/"))
        };
        let whole_tree =
            rel.is_none() && std::fs::canonicalize(cwd.join(f)).ok().as_deref() == Some(root);
        if whole_tree {
            changes.extend(tree.files.iter().map(|p| Change {
                path: p.clone(),
                status: Status::Modified,
                from: None,
            }));
            continue;
        }
        let rel = rel.ok_or_else(|| format!("{f} is outside the repository"))?;
        let prefix = format!("{rel}/");
        let under: Vec<&String> = tree
            .files
            .iter()
            .filter(|p| p.starts_with(&prefix))
            .collect();
        if tree.contains(&rel) {
            changes.push(Change {
                path: rel,
                status: Status::Modified,
                from: None,
            });
        } else if !under.is_empty() {
            changes.extend(under.into_iter().map(|p| Change {
                path: p.clone(),
                status: Status::Modified,
                from: None,
            }));
        } else {
            changes.push(Change {
                path: rel,
                status: Status::Deleted,
                from: None,
            });
        }
    }
    changes.sort();
    changes.dedup();
    Ok(changes)
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
    let scanned = match &changes.cache_from {
        Some(owner) => build_sharing(&root, &loaded.config, owner),
        None => build(&root, &loaded.config),
    };
    let mut scan = scanned.map_err(|e| format!("could not read {}: {e}", root.display()))?;
    let changes_everything = changes.everything.clone();
    let (full_suites, stage) = (changes.full_suites, changes.stage);
    let (changes, base) = if changes.files.is_empty() {
        let base = changes
            .base
            .clone()
            .or_else(|| default_base(&root, Some(&loaded.config)));
        let merge_base = match base {
            Some(base) => git::merge_base(&root, &base).map_err(|e| {
                format!("{e}\nno merge base with {base}: in a shallow clone, fetch more history (fetch-depth: 0) or pass --files")
            })?,
            None => {
                let head = git::head(&root)
                    .ok_or("no base branch found and no commit yet; pass --base or --files")?;
                eprintln!("{NO_BASE}");
                head
            }
        };
        (git::changes(&root, &merge_base)?, Some(merge_base))
    } else {
        (explicit(&root, &cwd, &scan.tree, &changes.files)?, None)
    };
    let clean = git::clean_tree_id(&root);
    let head = match &clean {
        Some(_) => git::head(&root).unwrap_or_else(|| "worktree".into()),
        None => "worktree".into(),
    };
    let tree_hash = clean.unwrap_or_else(|| digest::worktree_hash(&scan.tree));
    let base_files = match &base {
        Some(base) => fairlead_tests::planner::base_files(&root, base, &changes),
        None => Default::default(),
    };
    let input = Input {
        base_files,
        everything: changes_everything,
        full_suites,
        stage,
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

/// The plan of `changes`, or with `head`, of that commit's changes from a
/// temporary checkout of it, which lives as long as the returned guard.
fn make_at(
    cwd: &Path,
    changes: &Changes,
    head: Option<&str>,
) -> Result<(Planned, Option<crate::head::Checkout>), String> {
    let Some(rev) = head else {
        return make(cwd, changes).map(|p| (p, None));
    };
    if !changes.files.is_empty() {
        return Err("--head plans a commit's own changes, so it can't take --files".into());
    }
    let top = crate::head::top(cwd)?;
    let checkout = crate::head::check_out(&top, rev)?;
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let inner = cwd
        .strip_prefix(&top)
        .map(|rel| checkout.dir.join(rel))
        .ok()
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| checkout.dir.clone());
    let mut at = changes.clone();
    at.base = Some(crate::head::base(
        &top,
        &checkout.dir,
        changes.base.as_deref(),
    )?);
    at.cache_from = Some(top);
    let planned = make(&inner, &at)?;
    Ok((planned, Some(checkout)))
}

/// A workspace plans each repository at its own head, so `--head` is for one.
fn workspace_or_head(
    cwd: &Path,
    changes: &Changes,
    head: Option<&str>,
) -> Result<Option<fairlead_core::config::workspace::Found>, String> {
    let found = crate::workspace_cmd::resolve(cwd, changes)?;
    if found.is_some() && head.is_some() {
        return Err(
            "--head plans one repository: run it inside the repository, without --repo".into(),
        );
    }
    Ok(found)
}

pub fn run_plan(
    cwd: &Path,
    changes: Changes,
    head: Option<String>,
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
    match workspace_or_head(cwd, &changes, head.as_deref()) {
        Ok(Some(ws)) => return crate::workspace_cmd::run_plan(&ws, cwd, changes, json, out),
        Ok(None) => {}
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    }
    let (planned, _checkout) = match make_at(cwd, &changes, head.as_deref()) {
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

pub fn run_explain(cwd: &Path, changes: Changes, head: Option<&str>, target: &str) -> ExitCode {
    match workspace_or_head(cwd, &changes, head) {
        Ok(Some(ws)) => return crate::workspace_cmd::run_explain(&ws, cwd, changes, target),
        Ok(None) => {}
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    }
    let (planned, checkout) = match make_at(cwd, &changes, head) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    // The target names a path as the caller sees it, in their own checkout.
    let root = match &checkout {
        Some(_) => crate::head::top(cwd).unwrap_or_else(|_| planned.scan.tree.root.clone()),
        None => planned.scan.tree.root.clone(),
    };
    let abs = std::fs::canonicalize(cwd.join(target)).unwrap_or_else(|_| cwd.join(target));
    let rel =
        fairlead_lang::tree::relative(&root, &abs).unwrap_or_else(|| target.replace('\\', "/"));
    if let Some(text) = waiting_whole(cwd, &changes, head, &planned, &rel) {
        println!("{text}");
        return ExitCode::SUCCESS;
    }
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

/// The explanation for a test the change reaches whose whole suite waits,
/// from the same change planned with `--full`, so `explain` agrees with the
/// share `plan` and `done` print.
fn waiting_whole(
    cwd: &Path,
    changes: &Changes,
    head: Option<&str>,
    planned: &Planned,
    target: &str,
) -> Option<String> {
    let plan = &planned.plan;
    let waits = plan
        .deferred
        .iter()
        .any(|d| !d.why.is_stage() && d.reached > 0);
    if !waits || plan.tests.iter().any(|t| t.path == target) {
        return None;
    }
    let mut all = changes.clone();
    all.full_suites = true;
    let (full, _checkout) = make_at(cwd, &all, head).ok()?;
    fairlead_tests::explain::deferred(plan, &full.plan, &full.scan, target)
}
