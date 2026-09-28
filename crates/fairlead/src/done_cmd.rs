//! `fairlead done`: the gate a change passes before it counts as finished.
//! It plans the working tree, runs the planned tests and checks, the checks
//! `done.always` names and the guard, and records the outcome against the
//! tree it checked, so a result can never stand for a tree that has changed.

use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::Instant;

use fairlead_core::config::{DoneTests, OnStop};
use fairlead_core::plan::{Invocation, InvocationKind};
use fairlead_guard::events::{Event, EventLog, Step};
use fairlead_tests::git;

use crate::plan_cmd::{make, Changes};

#[derive(clap::Args)]
pub struct DoneArgs {
    #[command(flatten)]
    changes: Changes,
    /// Print the steps without running them.
    #[arg(long)]
    dry_run: bool,
    /// Run every step even after one fails.
    #[arg(long)]
    keep_going: bool,
    /// Only say whether the working tree as it stands has passed; exits 0 if it has.
    #[arg(long, conflicts_with_all = ["dry_run", "keep_going"])]
    check: bool,
}

pub fn run(args: DoneArgs, cwd: &Path) -> ExitCode {
    match gate(&args, cwd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("done: {e}");
            ExitCode::from(2)
        }
    }
}

fn gate(args: &DoneArgs, cwd: &Path) -> Result<ExitCode, String> {
    let planned = make(cwd, &args.changes)?;
    let root = crate::graph_cmd::repo_root(cwd);
    let tree = planned.plan.tree_hash.clone();
    let log = EventLog::open(&root);
    if args.check {
        let passed = log
            .as_ref()
            .is_some_and(|l| last_outcome(&l.read(), &tree) == Some(true));
        println!(
            "done: {} for this tree",
            if passed { "passed" } else { "not passed" }
        );
        return Ok(if passed {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let steps = steps(&planned)?;
    if args.dry_run {
        for s in &steps {
            println!("done: ({}) {}", s.cwd, s.argv.join(" "));
        }
        println!("done: {} steps", steps.len());
        return Ok(ExitCode::SUCCESS);
    }
    let started = Instant::now();
    let mut ran = Vec::new();
    for s in &steps {
        let at = Instant::now();
        println!("done: ({}) {}", s.cwd, s.argv.join(" "));
        let passed = run_step(&root, s);
        let seconds = (at.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        println!(
            "done: {} {} in {seconds} s",
            if passed { "passed" } else { "failed" },
            s.id
        );
        ran.push(Step {
            id: s.id.clone(),
            passed,
            seconds,
        });
        if !passed && !args.keep_going {
            break;
        }
    }
    let passed = ran.len() == steps.len() && ran.iter().all(|s| s.passed);
    if let Some(log) = &log {
        let mut event = Event::new(
            "done",
            if passed { "pass" } else { "fail" },
            started.elapsed(),
        );
        event.tree = Some(tree);
        event.steps = ran;
        // The log is a record; failing to write it never changes the outcome.
        let _ = log.append(&event);
    }
    if passed {
        println!("done: passed, {} steps", steps.len());
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("done: not passed; fix the failing step and run `fairlead done` again");
        Ok(ExitCode::FAILURE)
    }
}

/// The planned tests and checks `[done]` asks for, then `done.always`
/// checks the plan didn't select, then the guard.
fn steps(planned: &crate::plan_cmd::Planned) -> Result<Vec<Invocation>, String> {
    let done = &planned.config.done;
    let mut out: Vec<Invocation> = planned
        .plan
        .invocations
        .iter()
        .filter(|i| match i.kind {
            InvocationKind::Runner => done.tests == DoneTests::Planned,
            InvocationKind::Check => done.checks == DoneTests::Planned,
        })
        .cloned()
        .collect();
    let changed: Vec<String> = planned
        .plan
        .changed
        .iter()
        .filter(|c| c.status != fairlead_core::plan::Status::Deleted)
        .map(|c| c.path.clone())
        .collect();
    for id in done.always.items() {
        if out
            .iter()
            .any(|i| i.kind == InvocationKind::Check && &i.id == id)
        {
            continue;
        }
        let check = planned
            .config
            .checks
            .items()
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| format!("done.always names `{id}`, which isn't a [[checks]] id"))?;
        // Unplanned, `{files}` stands for the change's files.
        let argv = check
            .command
            .iter()
            .flat_map(|a| {
                if a == "{files}" {
                    changed.clone()
                } else {
                    vec![a.clone()]
                }
            })
            .collect();
        out.push(Invocation {
            id: check.id.clone(),
            kind: InvocationKind::Check,
            cwd: ".".into(),
            argv,
        });
    }
    if done.guard {
        let me = std::env::current_exe().map_err(|e| format!("can't find this binary: {e}"))?;
        out.push(Invocation {
            id: "guard".into(),
            kind: InvocationKind::Check,
            cwd: ".".into(),
            argv: vec![
                me.to_string_lossy().into_owned(),
                "guard".into(),
                "check".into(),
            ],
        });
    }
    Ok(out)
}

fn run_step(root: &Path, step: &Invocation) -> bool {
    let Some((program, args)) = step.argv.split_first() else {
        eprintln!("done: {} has no command", step.id);
        return false;
    };
    match Command::new(program)
        .args(args)
        .current_dir(root.join(&step.cwd))
        .status()
    {
        Ok(status) => status.success(),
        Err(e) => {
            eprintln!("done: couldn't start {program}: {e}");
            false
        }
    }
}

/// The Claude Code Stop hook. It never runs the gate itself, which would
/// outlast the hook's timeout: it reads whether the tree the agent leaves
/// has passed, and on any error lets the stop through.
pub fn stop() -> ExitCode {
    let mut input = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut input);
    let call: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let dir = call["cwd"]
        .as_str()
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let asked = call["stop_hook_active"].as_bool().unwrap_or(false);
    match stop_reason(&dir, asked) {
        Some(reason) => {
            eprintln!("{reason}");
            ExitCode::from(2)
        }
        None => ExitCode::SUCCESS,
    }
}

/// Why the agent should keep going, or none to let it stop.
fn stop_reason(dir: &Path, asked: bool) -> Option<String> {
    let loaded = fairlead_core::config::load(
        dir,
        &fairlead_core::config::LoadOptions::from_process(Vec::new()),
    )
    .ok()?;
    let on_stop = loaded.config.done.on_stop;
    if on_stop == OnStop::Off || (on_stop == OnStop::Ask && asked) {
        return None;
    }
    let root = crate::graph_cmd::repo_root(dir);
    let base = git::default_base(&root).ok()?;
    let merge_base = git::merge_base(&root, &base).ok()?;
    // Nothing changed means nothing to gate, as for a session that only read.
    if git::changes(&root, &merge_base).ok()?.is_empty() {
        return None;
    }
    let tree = tree_hash(&root)?;
    let log = EventLog::open(&root)?;
    let why = match last_outcome(&log.read(), &tree) {
        Some(true) => return None,
        Some(false) => "its last `fairlead done` failed",
        None => "`fairlead done` hasn't passed for the tree as it is now",
    };
    let mut event = Event::new("stop", "block", std::time::Duration::ZERO);
    event.tree = Some(tree);
    let _ = log.append(&event);
    Some(format!(
        "fairlead: this change isn't finished: {why}. Run `fairlead done` and fix what it reports before stopping."
    ))
}

/// The plan's tree hash for the working tree as it stands.
fn tree_hash(root: &Path) -> Option<String> {
    if let Some(id) = git::clean_tree_id(root) {
        return Some(id);
    }
    let root = std::fs::canonicalize(root).ok()?;
    let tree = fairlead_lang::tree::Tree::scan(&fairlead_lang::tree::plain(&root));
    Some(fairlead_tests::digest::worktree_hash(&tree))
}

/// Whether the newest `done` run for `tree` passed, if one ran.
pub fn last_outcome(log: &str, tree: &str) -> Option<bool> {
    log.lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|e| e["stage"] == "done" && e["tree"] == tree)
        .map(|e| e["decision"] == "pass")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_done_for_a_tree_decides_and_other_trees_are_ignored() {
        let log = [
            r#"{"stage":"done","tree":"t1","decision":"fail"}"#,
            r#"{"stage":"write","decision":"deny"}"#,
            r#"{"stage":"done","tree":"t1","decision":"pass"}"#,
            r#"{"stage":"done","tree":"t2","decision":"fail"}"#,
            "not json",
        ]
        .join("\n");
        assert_eq!(last_outcome(&log, "t1"), Some(true));
        assert_eq!(last_outcome(&log, "t2"), Some(false));
        assert_eq!(last_outcome(&log, "t3"), None);
    }
}
