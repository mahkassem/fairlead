//! `fairlead plan` and `test --explain` from a workspace folder: each
//! repository planned against its own base, and one plan of those that
//! changed. Everything else runs inside one repository.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::config::workspace::{self, Found, Member};
use fairlead_tests::render;
use serde::Serialize;

use crate::plan_cmd::{make, Changes};

/// The workspace a command from `cwd` plans across: the one `cwd` is the
/// folder of, or with `--repo`, the one holding the repository `cwd` is in.
pub fn resolve(cwd: &Path, changes: &Changes) -> Result<Option<Found>, String> {
    let found = workspace::at(cwd).map_err(|e| e.to_string())?;
    if found.is_some() || changes.repo.is_none() {
        return Ok(found);
    }
    let held = match workspace::git_root(cwd) {
        Some(repo) => workspace::enclosing(&repo).map_err(|e| e.to_string())?,
        None => None,
    };
    held.map(Some)
        .ok_or_else(|| "--repo names a repository of a workspace, and this isn't in one".into())
}

/// One repository's changes: its own base unless `--base` names one, and
/// only the `--files` inside it.
fn changes_for(member: &Member, changes: &Changes, files: Vec<String>) -> Changes {
    let base = changes.base.clone().or_else(|| member.base.clone());
    let mut own = Changes::new(base, files, changes.sets.clone());
    own.everything = changes.everything.clone();
    own
}

/// `--files` by the repository holding each, as absolute paths.
fn files_by_repo(
    ws: &Found,
    cwd: &Path,
    files: &[String],
) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut by: Vec<(String, Vec<String>)> = Vec::new();
    for f in files {
        let given = Path::new(f);
        let abs = if given.is_absolute() {
            given.to_path_buf()
        } else {
            cwd.join(given)
        };
        let abs = std::fs::canonicalize(&abs).unwrap_or(abs);
        let member = ws
            .holding(&abs)
            .ok_or_else(|| format!("{f} is in none of the workspace's repositories"))?;
        let path = abs.to_string_lossy().into_owned();
        match by.iter_mut().find(|(n, _)| *n == member.name) {
            Some((_, list)) => list.push(path),
            None => by.push((member.name.clone(), vec![path])),
        }
    }
    Ok(by)
}

#[derive(Serialize)]
struct RepoPlan<'a> {
    name: &'a str,
    /// From the workspace folder.
    path: &'a str,
    plan: fairlead_core::plan::Plan,
}

#[derive(Serialize)]
struct WorkspacePlan<'a> {
    repos: Vec<RepoPlan<'a>>,
    /// The repositories with nothing changed, which the plan leaves out.
    unchanged: Vec<&'a str>,
}

fn members<'a>(ws: &'a Found, changes: &Changes) -> Result<Vec<&'a Member>, String> {
    match &changes.repo {
        Some(name) => Ok(vec![ws.member(name)?]),
        None => Ok(ws.repos.iter().collect()),
    }
}

pub fn run_plan(
    ws: &Found,
    cwd: &Path,
    changes: Changes,
    json: bool,
    out: Option<PathBuf>,
) -> ExitCode {
    match plan(ws, cwd, &changes) {
        Ok(planned) => {
            let text = serde_json::to_string_pretty(&planned).expect("plan prints");
            if let Some(path) = out {
                if let Err(e) = std::fs::write(&path, format!("{text}\n")) {
                    eprintln!("could not write {}: {e}", path.display());
                    return ExitCode::from(2);
                }
            }
            if json {
                println!("{text}");
            } else {
                print!("{}", render_text(&planned));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}

fn plan<'a>(ws: &'a Found, cwd: &Path, changes: &Changes) -> Result<WorkspacePlan<'a>, String> {
    let chosen = members(ws, changes)?;
    let files = if changes.files.is_empty() {
        None
    } else {
        Some(files_by_repo(ws, cwd, &changes.files)?)
    };
    let mut planned = WorkspacePlan {
        repos: Vec::new(),
        unchanged: Vec::new(),
    };
    for member in chosen {
        let own = match &files {
            None => Vec::new(),
            Some(by) => match by.iter().find(|(n, _)| *n == member.name) {
                Some((_, list)) => list.clone(),
                None => {
                    planned.unchanged.push(&member.name);
                    continue;
                }
            },
        };
        let made = make(&member.dir, &changes_for(member, changes, own))
            .map_err(|e| format!("{}: {e}", member.name))?;
        if made.plan.changed.is_empty() && made.plan.ignored.is_empty() && !made.plan.all {
            planned.unchanged.push(&member.name);
        } else {
            planned.repos.push(RepoPlan {
                name: &member.name,
                path: &member.path,
                plan: made.plan,
            });
        }
    }
    Ok(planned)
}

fn render_text(planned: &WorkspacePlan) -> String {
    let mut out = String::new();
    for repo in &planned.repos {
        out.push_str(&format!("{} ({})\n", repo.name, repo.path));
        for line in render::text(&repo.plan).lines() {
            out.push_str(&format!("  {line}\n"));
        }
        out.push('\n');
    }
    if !planned.unchanged.is_empty() {
        out.push_str(&format!("unchanged: {}\n", planned.unchanged.join(", ")));
    }
    out
}

/// `test --explain` from a workspace folder: the repository `--repo` names,
/// else the one holding the target.
pub fn run_explain(ws: &Found, cwd: &Path, changes: Changes, target: &str) -> ExitCode {
    let abs = cwd.join(target);
    let abs = std::fs::canonicalize(&abs).unwrap_or(abs);
    let member = match &changes.repo {
        Some(name) => ws.member(name),
        None => ws.holding(&abs).ok_or_else(|| {
            format!(
                "{target} is in none of the workspace's repositories; name one with --repo ({})",
                ws.names()
            )
        }),
    };
    let member = match member {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let target = if abs.exists() {
        abs.to_string_lossy().into_owned()
    } else {
        target.to_string()
    };
    let files = match files_by_repo(ws, cwd, &changes.files) {
        Ok(by) => by
            .into_iter()
            .find(|(n, _)| *n == member.name)
            .map(|(_, list)| list)
            .unwrap_or_default(),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let own = changes_for(member, &changes, files);
    crate::plan_cmd::run_explain(&member.dir, own, &target)
}
