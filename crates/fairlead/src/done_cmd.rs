//! `fairlead done`: the gate a change passes before it counts as finished.
//! It plans the working tree, runs the planned tests and checks, the checks
//! `done.always` names and the guard, and records the outcome against the
//! tree it checked, so a result can never stand for a tree that has changed.

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use fairlead_core::config::{DoneTests, OnStop, Profile};
use fairlead_core::plan::{Invocation, InvocationKind, Quarantined};
use fairlead_guard::events::{Event, EventLog, Step, Waiting};
use fairlead_tests::{git, quarantine};

use crate::done_record::{self as record, State};
use crate::plan_cmd::{make, Changes};
use crate::step::Outcome;

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
    /// Only say whether the working tree as it stands has passed, and if not, why
    /// and what to run; exits 0 if it has.
    #[arg(long, conflicts_with_all = ["dry_run", "keep_going"])]
    check: bool,
    /// Run whole suites that wait for a later stage too.
    #[arg(long, conflicts_with = "check")]
    full: bool,
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
    let mut changes = args.changes.clone();
    changes.full_suites = args.full;
    let planning = Instant::now();
    let planned = make(cwd, &changes)?;
    let plan_seconds = (planning.elapsed().as_secs_f64() * 100.0).round() / 100.0;
    let root = crate::graph_cmd::repo_root(cwd);
    let tree = planned.plan.tree_hash.clone();
    let log = EventLog::open(&root);
    if args.check {
        let text = log.as_ref().map(|l| l.read()).unwrap_or_default();
        let state = record::state(record::last_done(&text, &tree).as_ref());
        println!("done: {}", check_line(&state, &planned, &root));
        return Ok(if state == State::Passed {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let (steps, skips) = crate::done_needs::apply(&planned, steps(&planned)?, &root)?;
    for skip in &skips {
        println!("done: {}", skip.line());
    }
    if args.dry_run {
        for s in &steps {
            println!("done: ({}) {}", s.cwd, s.argv.join(" "));
        }
        println!("done: {} steps", steps.len());
        return Ok(ExitCode::SUCCESS);
    }
    let started = Instant::now();
    let ran = run_steps(&planned, &steps, &root, args.keep_going);
    let unproved = skips.iter().find(|s| s.proved_in.is_none());
    let passed = ran.len() == steps.len()
        && ran.iter().all(|s| s.passed || s.quarantined)
        && unproved.is_none();
    let not_provable = ran.iter().filter(|s| s.quarantined).count();
    let all_ran = ran.len() == steps.len();
    let waiting = waiting(&planned);
    let decision = match (passed, waiting.is_empty()) {
        (false, _) => "fail",
        (true, true) => "pass",
        (true, false) => "deferred",
    };
    if let Some(log) = &log {
        let mut event = Event::new("done", decision, started.elapsed());
        event.tree = Some(tree);
        event.steps = ran;
        event.waiting = waiting.clone();
        event.unmet = skips.iter().map(|s| s.record()).collect();
        event.plan_seconds = Some(plan_seconds);
        // The log is a record; failing to write it never changes the outcome.
        let _ = log.append(&event);
    }
    let deferred = planned
        .plan
        .deferred
        .iter()
        .filter(|_| !guard_only(&planned));
    for d in deferred.filter(|d| !d.why.is_stage()) {
        println!("done: {}", d.advice());
    }
    let elsewhere = match skips.len() {
        0 => String::new(),
        _ => "; what an unmet need covers wasn't run here and is proved where it says".into(),
    };
    if passed && !waiting.is_empty() {
        println!(
            "done: passed, {} steps{elsewhere}; not finished: {}",
            steps.len(),
            record::hold(&waiting, &planned.config, &root)
        );
        Ok(ExitCode::SUCCESS)
    } else if passed && not_provable > 0 {
        println!(
            "done: passed, {} steps{elsewhere}; {not_provable} failed as quarantined, so not provable here: say so in the pull request",
            steps.len()
        );
        Ok(ExitCode::SUCCESS)
    } else if passed {
        println!("done: passed, {} steps{elsewhere}", steps.len());
        Ok(ExitCode::SUCCESS)
    } else if let (Some(skip), true) = (unproved, all_ran) {
        eprintln!(
            "done: not passed: {}",
            record::unmet_line(&skip.need, &skip.step)
        );
        Ok(ExitCode::FAILURE)
    } else {
        eprintln!("done: not passed; fix the failing step and run `fairlead done` again");
        Ok(ExitCode::FAILURE)
    }
}

/// What `done --check` says about the tree.
fn check_line(state: &State, planned: &crate::plan_cmd::Planned, root: &Path) -> String {
    match state {
        State::Passed => "passed for this tree".to_string(),
        State::Waits(waiting) => format!(
            "not finished for this tree: its steps passed, but {}",
            record::hold(waiting, &planned.config, root)
        ),
        State::Unmet { need, step } => {
            format!("not passed for this tree: {}", record::unmet_line(need, step))
        }
        State::Failed(step) => format!(
            "not passed for this tree: its last run failed{}; fix that and run `fairlead done` again",
            step.as_ref().map(|s| format!(" at `{s}`")).unwrap_or_default()
        ),
        State::NotRun => "not passed for this tree: `fairlead done` hasn't run on the tree as it is now; run `fairlead done`".to_string(),
    }
}

/// Runs the steps in order, each saying how it went, until one fails
/// unless `keep_going`.
fn run_steps(
    planned: &crate::plan_cmd::Planned,
    steps: &[Invocation],
    root: &Path,
    keep_going: bool,
) -> Vec<Step> {
    let held = held(planned, steps);
    let mut ran = Vec::new();
    for s in steps {
        let at = Instant::now();
        println!("done: ({}) {}", s.cwd, s.argv.join(" "));
        let entry = s.quarantined.as_ref().and_then(|target| {
            held.iter()
                .find(|q| q.kind == s.kind && &q.target == target)
        });
        let (outcome, _, why) = crate::step::run_saying_why("done", root, s, entry, true);
        let seconds = (at.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        let (word, name) = match (outcome, entry) {
            (Outcome::Passed, _) => ("passed", s.id.clone()),
            (Outcome::Held, Some(q)) => ("not provable here:", format!("{} ({})", s.id, q.target)),
            _ => ("failed", s.id.clone()),
        };
        let why = why.map(|w| format!(": {w}")).unwrap_or_default();
        println!("done: {word} {name} in {seconds} s{why}");
        ran.push(Step {
            id: s.id.clone(),
            passed: outcome == Outcome::Passed,
            quarantined: outcome == Outcome::Held,
            seconds,
            stage: Some(
                crate::stage::from_of(&planned.config, &s.id, &s.kind)
                    .name()
                    .into(),
            ),
            os: Some(std::env::consts::OS.into()),
            arch: Some(std::env::consts::ARCH.into()),
        });
        if outcome == Outcome::Failed && !keep_going {
            break;
        }
    }
    ran
}

/// Whole suites the change reaches most of, which hold the gate open until a
/// run with `--full`: once anything is deployed. Before that, the scheduled
/// run is enough.
fn waiting(planned: &crate::plan_cmd::Planned) -> Vec<Waiting> {
    let early = planned.config.phase == Some(fairlead_core::config::Phase::Early);
    if early || guard_only(planned) {
        return Vec::new();
    }
    planned
        .plan
        .deferred
        .iter()
        .filter(|d| !d.why.is_stage() && d.reached > 0)
        .map(|d| Waiting {
            id: d.id.clone(),
            reached: d.reached,
            selected: d.selected,
            by_rule: d.by_rule,
        })
        .collect()
}

fn guard_only(planned: &crate::plan_cmd::Planned) -> bool {
    planned.config.profile == Profile::Guard
}

/// The planned tests and checks `[done]` asks for, then `done.always`
/// checks the plan didn't select, then the guard and the `[pr]` rules. The
/// guard-only profile runs only the last two.
pub fn steps(planned: &crate::plan_cmd::Planned) -> Result<Vec<Invocation>, String> {
    let mut out = if guard_only(planned) {
        Vec::new()
    } else {
        planned_steps(planned)?
    };
    let me = || -> Result<String, String> {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| format!("can't find this binary: {e}"))
    };
    let mut fairlead = |id: &str, args: &[&str]| -> Result<(), String> {
        let mut argv = vec![me()?];
        argv.extend(args.iter().map(|a| a.to_string()));
        out.push(Invocation {
            id: id.into(),
            kind: InvocationKind::Check,
            cwd: ".".into(),
            argv,
            quarantined: None,
        });
        Ok(())
    };
    // A diff rule counts only what the change adds, so it needs the base.
    let diffs = planned.config.guard.external.items().iter().any(|e| e.diff);
    let base = planned.plan.base.as_deref().filter(|_| diffs);
    if planned.config.done.guard || guard_only(planned) {
        match base {
            Some(base) => fairlead("guard", &["guard", "check", "--base", base])?,
            None => fairlead("guard", &["guard", "check"])?,
        }
    }
    if planned.config.pr.is_some() {
        fairlead("pr", &["pr", "check", "--local"])?;
    }
    Ok(out)
}

/// The planned tests and checks `[done]` asks for, then `done.always`
/// checks the plan didn't select.
fn planned_steps(planned: &crate::plan_cmd::Planned) -> Result<Vec<Invocation>, String> {
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
        let root = &planned.scan.tree.root;
        let held = quarantine::check_held(&planned.config, root, id).map(|q| q.target);
        out.push(Invocation {
            id: check.id.clone(),
            kind: InvocationKind::Check,
            cwd: ".".into(),
            argv,
            quarantined: held,
        });
    }
    Ok(out)
}

/// The entries holding the steps: the plan's, and those holding a
/// `done.always` check the plan didn't select.
fn held(planned: &crate::plan_cmd::Planned, steps: &[Invocation]) -> Vec<Quarantined> {
    let mut out = planned.plan.quarantined.clone();
    for s in steps.iter().filter(|s| s.quarantined.is_some()) {
        if planned.plan.quarantine_of(s).is_none() {
            let root = &planned.scan.tree.root;
            out.extend(quarantine::check_held(&planned.config, root, &s.id));
        }
    }
    out
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
    crate::tokens::record_from_hook(&dir, &call);
    let transcript = call["transcript_path"].as_str().map(Path::new);
    match stop_reason(&dir, asked, call["session_id"].as_str(), transcript) {
        Some(reason) => {
            eprintln!("{reason}");
            ExitCode::from(2)
        }
        None => ExitCode::SUCCESS,
    }
}

/// Why the agent should keep going, or none to let it stop. Under `ask` it
/// sends a session back once per tree: Claude Code clears
/// `stop_hook_active` every turn, so that alone asks again each turn.
fn stop_reason(
    dir: &Path,
    asked: bool,
    session: Option<&str>,
    transcript: Option<&Path>,
) -> Option<String> {
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
    let log = EventLog::open(&root)?;
    let text = log.read();
    // A session that edited nothing, such as a nested `claude -p` asked a
    // question, has nothing to finish. Its transcript says so best; without
    // one, the write log does; without either it can't tell.
    let logged = loaded.config.guard.events == fairlead_core::config::Events::Local;
    let edited = match transcript.and_then(may_have_edited) {
        Some(edited) => edited,
        None => session
            .filter(|_| logged)
            .is_none_or(|s| edited_since_pass(&text, s)),
    };
    if !edited {
        return None;
    }
    let merge_base = crate::plan_cmd::merge_base_or_head(&root, None, Some(&loaded.config))?;
    // Nothing changed means nothing to gate, as for a session that only read.
    if git::changes(&root, &merge_base).ok()?.is_empty() {
        return None;
    }
    let tree = tree_hash(&root)?;
    let ask = on_stop == OnStop::Ask;
    if ask && asked_before(&text, &tree, session) {
        return None;
    }
    let why = match record::state(record::last_done(&text, &tree).as_ref()) {
        State::Passed => return None,
        State::Waits(waiting) => format!(
            "the last `fairlead done` passed its steps, but {}",
            record::hold(&waiting, &loaded.config, &root)
        ),
        State::Unmet { need, step } => format!("{}.", record::unmet_line(&need, &step)),
        State::Failed(step) => format!(
            "its last `fairlead done` failed{}. Fix that and run `fairlead done` again before stopping.",
            step.map(|s| format!(" at `{s}`")).unwrap_or_default()
        ),
        State::NotRun => "`fairlead done` hasn't run on the tree as it is now. Run `fairlead done` and fix what it reports before stopping.".to_string(),
    };
    let mut event = Event::new("stop", "block", std::time::Duration::ZERO);
    event.tree = Some(tree);
    event.session = session.map(String::from);
    let _ = log.append(&event);
    let once = if ask {
        " Under `done.on_stop = \"ask\"` this is the one reminder for this tree in this session."
    } else {
        ""
    };
    Some(format!("fairlead: this change isn't finished: {why}{once}"))
}

/// Tools that can change files. A shell command can, and an agent the
/// session starts can use any of them.
const EDITING: [&str; 7] = [
    "Edit",
    "Write",
    "MultiEdit",
    "NotebookEdit",
    "Bash",
    "Task",
    "Agent",
];

/// Whether the session's transcript shows a tool use that can change files,
/// or none when it can't be read. A line that doesn't parse is skipped.
fn may_have_edited(transcript: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(transcript).ok()?;
    Some(
        text.lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .any(|e| {
                e["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|c| {
                        c["type"] == "tool_use"
                            && c["name"].as_str().is_some_and(|n| EDITING.contains(&n))
                    })
            }),
    )
}

/// Whether the write hook logged an edit by `session` after the newest
/// passing `done` run. A shell command is logged only when refused, and a
/// refused one changed nothing.
fn edited_since_pass(log: &str, session: &str) -> bool {
    let events: Vec<serde_json::Value> = log
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let since = events
        .iter()
        .rposition(|e| e["stage"] == "done" && e["decision"] == "pass")
        .map_or(0, |i| i + 1);
    events[since..].iter().any(|e| {
        let shell = matches!(e["tool"].as_str(), Some("Bash" | "run_shell_command"));
        e["stage"] == "write" && e["session"] == session && (e["file"].is_string() || !shell)
    })
}

/// Whether the hook already sent `session` back for `tree`.
fn asked_before(log: &str, tree: &str, session: Option<&str>) -> bool {
    log.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .any(|e| e["stage"] == "stop" && e["tree"] == tree && e["session"].as_str() == session)
}

/// The plan's tree hash for the working tree as it stands.
pub fn tree_hash(root: &Path) -> Option<String> {
    if let Some(id) = git::clean_tree_id(root) {
        return Some(id);
    }
    let root = std::fs::canonicalize(root).ok()?;
    let tree = fairlead_lang::tree::Tree::scan(&fairlead_lang::tree::plain(&root));
    Some(fairlead_tests::digest::worktree_hash(&tree))
}
