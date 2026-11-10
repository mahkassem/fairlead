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
    /// Run the whole plan, and whole suites that wait for a later stage too,
    /// whatever `done.budget_seconds` says.
    #[arg(long, conflicts_with = "check")]
    full: bool,
    /// Run what fits in this many seconds and owe the rest to CI, in place
    /// of `done.budget_seconds`; 0 runs the whole plan.
    #[arg(long, value_name = "SECONDS", conflicts_with_all = ["check", "full"])]
    budget: Option<u32>,
    /// The pre-push hook: the plan, as `done` runs it, then the push checks
    /// in place of `pr check`; a tree that has passed runs the push checks only.
    #[arg(long, conflicts_with = "check")]
    pre_push: bool,
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
    let budget = budget_of(args, cwd, &changes);
    // Under a budget nothing waits for a later stage: what doesn't fit is owed.
    changes.full_suites = args.full || budget > 0;
    let planning = Instant::now();
    let mut planned = make(cwd, &changes)?;
    let root = crate::graph_cmd::repo_root(cwd);
    let tree = planned.plan.tree_hash.clone();
    let log = EventLog::open(&root);
    let text = log.as_ref().map(|l| l.read()).unwrap_or_default();
    if args.check {
        let state = record::state(record::last_done(&text, &tree).as_ref());
        println!("done: {}", check_line(&state, &planned, &root));
        return Ok(if state == State::Passed {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    if args.pre_push && record::state(record::last_done(&text, &tree).as_ref()) == State::Passed {
        return Ok(push_only(&planned, &root));
    }
    let suites = crate::done_budget::Suites::read(&planned)?;
    let narrow = narrow_plan(budget, cwd, &changes, &planned);
    if budget > 0 {
        planned.plan.invocations = crate::done_budget::named(&planned, &suites);
    }
    let plan_seconds = (planning.elapsed().as_secs_f64() * 100.0).round() / 100.0;
    let (planned_steps, why) = steps_saying_why(&planned, args.pre_push)?;
    for line in &why {
        println!("done: {line}");
    }
    let (steps, skips) = crate::done_needs::apply(&planned, planned_steps, &root)?;
    for skip in &skips {
        println!("done: {}", skip.line());
    }
    let near = narrow.as_ref().unwrap_or(&planned.plan);
    let (steps, mut owed) = within(budget, &planned, near, &suites, steps, &text)?;
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
    owed.advice
        .extend(crate::done_budget::slow(&ran, owed.budget));
    if let Some(log) = &log {
        let named: usize = ran.iter().filter_map(|s| s.files).sum();
        let mut event = Event::new("done", decision, started.elapsed());
        event.tree = Some(tree);
        event.steps = ran;
        event.waiting = waiting.clone();
        event.unmet = skips.iter().map(|s| s.record()).collect();
        event.plan_seconds = Some(plan_seconds);
        event.budget_seconds = (owed.budget > 0).then_some(owed.budget);
        event.debts = owed.debts.clone();
        event.reach = Some(crate::done_budget::reach(
            &planned,
            &suites,
            owed.ran.unwrap_or(named),
        ));
        // The log is a record; failing to write it never changes the outcome.
        let _ = log.append(&event);
    }
    let outcome = Ending {
        passed,
        all_ran,
        not_provable,
        steps: steps.len(),
    };
    Ok(report(&planned, &root, &outcome, &waiting, &skips, &owed))
}

/// The plan without `run_all` and the unreached widening, which says how
/// near each test is to the change; none when the plan didn't widen.
fn narrow_plan(
    budget: u32,
    cwd: &Path,
    changes: &Changes,
    planned: &crate::plan_cmd::Planned,
) -> Option<fairlead_core::plan::Plan> {
    if budget == 0 || !crate::done_budget::widened(&planned.plan) {
        return None;
    }
    let mut near = changes.clone();
    near.sets.push("plan.run_all={ replace = [] }".into());
    near.sets.push("tests.unreached=\"warn\"".into());
    make(cwd, &near).ok().map(|p| p.plan)
}

/// The steps cut to the budget, with what that leaves owed; as they are
/// with no budget.
fn within(
    budget: u32,
    planned: &crate::plan_cmd::Planned,
    narrow: &fairlead_core::plan::Plan,
    suites: &crate::done_budget::Suites,
    steps: Vec<Invocation>,
    log: &str,
) -> Result<(Vec<Invocation>, Owed), String> {
    if budget == 0 {
        return Ok((steps, Owed::default()));
    }
    let root = &planned.scan.tree.root;
    let classes = crate::done_budget::classes(planned, narrow, suites)?;
    let inputs = crate::done_budget::Inputs {
        planned,
        suites,
        classes: &classes,
        per_file: &crate::done_budget::seconds_per_file(log),
        history: &crate::done_history::read(root, &planned.config, log),
        budget,
    };
    let cut = crate::done_budget::cut(&inputs, steps);
    for line in &cut.lines {
        println!("done: {line}");
    }
    let owed = Owed {
        budget,
        debts: cut.debts,
        ran: Some(cut.ran),
        advice: crate::done_budget::advice(planned, suites, &classes),
    };
    Ok((cut.steps, owed))
}

/// How a run went, for its last lines.
struct Ending {
    passed: bool,
    all_ran: bool,
    not_provable: usize,
    steps: usize,
}

/// The run's last lines: what waits, the advice, and whether it passed
/// with what it owes or left elsewhere.
fn report(
    planned: &crate::plan_cmd::Planned,
    root: &Path,
    outcome: &Ending,
    waiting: &[Waiting],
    skips: &[crate::done_needs::Skip],
    owed: &Owed,
) -> ExitCode {
    let deferred = planned
        .plan
        .deferred
        .iter()
        .filter(|_| !guard_only(planned));
    for d in deferred.filter(|d| !d.why.is_stage()) {
        println!("done: {}", d.advice());
    }
    owed.say();
    let elsewhere = format!(
        "{}{}",
        environment(skips),
        crate::done_budget::owed_line(&owed.debts)
    );
    let unproved = skips.iter().find(|s| s.proved_in.is_none());
    let steps = outcome.steps;
    if outcome.passed && !waiting.is_empty() {
        println!(
            "done: passed, {steps} steps{elsewhere}; not finished: {}",
            record::hold(waiting, &planned.config, root)
        );
        ExitCode::SUCCESS
    } else if outcome.passed && outcome.not_provable > 0 {
        println!(
            "done: passed, {steps} steps{elsewhere}; {} failed as quarantined, so not provable here: say so in the pull request",
            outcome.not_provable
        );
        ExitCode::SUCCESS
    } else if outcome.passed {
        println!("done: passed, {steps} steps{elsewhere}");
        ExitCode::SUCCESS
    } else if let (Some(skip), true) = (unproved, outcome.all_ran) {
        eprintln!(
            "done: not passed: {}",
            record::unmet_line(&skip.need, &skip.step)
        );
        ExitCode::FAILURE
    } else {
        eprintln!("done: not passed; fix the failing step and run `fairlead done` again");
        eprintln!("done: {}", crate::report_cmd::MISTAKE);
        ExitCode::FAILURE
    }
}

/// The budget a run is under, in seconds: none with `--full`, in the
/// guard-only profile, or when the gate runs no tests.
fn budget_of(args: &DoneArgs, cwd: &Path, changes: &Changes) -> u32 {
    if args.full {
        return 0;
    }
    let Ok(config) = crate::plan_cmd::config_for(cwd, changes) else {
        return 0;
    };
    if config.profile == Profile::Guard || config.done.tests != DoneTests::Planned {
        return 0;
    }
    args.budget.unwrap_or(config.done.budget_seconds)
}

/// What a budgeted run owes to CI, and the advice its plan backs.
#[derive(Default)]
struct Owed {
    budget: u32,
    debts: Vec<fairlead_guard::events::Debt>,
    ran: Option<usize>,
    advice: Vec<String>,
}

impl Owed {
    fn say(&self) {
        for line in &self.advice {
            println!("done: advice: {line}");
        }
        if !self.advice.is_empty() {
            println!("done: advice is for the pull request's notes or a follow-up, not for this change, unless it is what you were asked to do");
        }
    }
}

/// The summary's account of what the machine, not the change, kept from
/// running: "your change clean; environment: 2 skipped (postgres: proved
/// in CI)".
fn environment(skips: &[crate::done_needs::Skip]) -> String {
    if skips.is_empty() {
        return String::new();
    }
    let mut needs: Vec<String> = Vec::new();
    for s in skips {
        let line = match &s.proved_in {
            Some(at) => format!("{}: proved in {at}", s.need),
            None => format!("{}: proved nowhere yet", s.need),
        };
        if !needs.contains(&line) {
            needs.push(line);
        }
    }
    format!(
        "; your change clean; environment: {} skipped ({})",
        skips.len(),
        needs.join("; ")
    )
}

/// Before a push of a tree that has passed: only the push checks run, and
/// nothing is recorded, since the tree's record stands.
fn push_only(planned: &crate::plan_cmd::Planned, root: &Path) -> ExitCode {
    println!("done: passed for this tree already, so only the push checks run");
    if !crate::hooks_cmd::push_checks(&planned.config) {
        return ExitCode::SUCCESS;
    }
    let Ok(me) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    let step = Invocation {
        id: "push".into(),
        kind: InvocationKind::Check,
        cwd: ".".into(),
        argv: vec![
            me.to_string_lossy().into_owned(),
            "guard".into(),
            "check".into(),
            "--push".into(),
        ],
        quarantined: None,
    };
    let ran = run_steps(planned, &[step], root, false);
    match ran.iter().all(|s| s.passed) {
        true => ExitCode::SUCCESS,
        false => ExitCode::FAILURE,
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
            files: Some(crate::done_needs::tests_of(planned, s).len()).filter(|n| *n > 0),
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

/// The steps `done` runs, as `steps_saying_why` orders them.
pub fn steps(planned: &crate::plan_cmd::Planned) -> Result<Vec<Invocation>, String> {
    steps_saying_why(planned, false).map(|(steps, _)| steps)
}

/// The planned tests and checks `[done]` asks for, then `done.always`
/// checks the plan didn't select, then the guard and the `[pr]` rules, or
/// before a push the push checks. The guard-only profile runs only the
/// last two. CI history puts cheap steps it shows red first, and adds cheap
/// checks it shows red, each with a line saying why.
pub fn steps_saying_why(
    planned: &crate::plan_cmd::Planned,
    pre_push: bool,
) -> Result<(Vec<Invocation>, Vec<String>), String> {
    let (mut out, why) = if guard_only(planned) {
        (Vec::new(), Vec::new())
    } else {
        let root = &planned.scan.tree.root;
        let log = EventLog::open(root).map(|l| l.read()).unwrap_or_default();
        let history = crate::done_history::read(root, &planned.config, &log);
        crate::done_history::coach(&planned.config, &history, planned_steps(planned)?, |id| {
            unplanned_check(planned, id)
        })?
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
    let push = crate::hooks_cmd::push_checks(&planned.config);
    match (pre_push, push) {
        (true, true) => fairlead("push", &["guard", "check", "--push"])?,
        (true, false) => {}
        (false, _) if planned.config.pr.is_some() => fairlead("pr", &["pr", "check", "--local"])?,
        (false, _) => {}
    }
    Ok((out, why))
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
    for id in done.always.items() {
        if out
            .iter()
            .any(|i| i.kind == InvocationKind::Check && &i.id == id)
        {
            continue;
        }
        let check = unplanned_check(planned, id)
            .map_err(|_| format!("done.always names `{id}`, which isn't a [[checks]] id"))?;
        out.push(check);
    }
    Ok(out)
}

/// A `[[checks]]` entry the plan didn't select, as a step; `{files}`
/// stands for the change's files.
fn unplanned_check(planned: &crate::plan_cmd::Planned, id: &str) -> Result<Invocation, String> {
    let changed: Vec<String> = planned
        .plan
        .changed
        .iter()
        .filter(|c| c.status != fairlead_core::plan::Status::Deleted)
        .map(|c| c.path.clone())
        .collect();
    let check = planned
        .config
        .checks
        .items()
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("`{id}` isn't a [[checks]] id"))?;
    let argv = check
        .command
        .iter()
        .flat_map(|a| match a == "{files}" {
            true => changed.clone(),
            false => vec![a.clone()],
        })
        .collect();
    let root = &planned.scan.tree.root;
    Ok(Invocation {
        id: check.id.clone(),
        kind: InvocationKind::Check,
        cwd: ".".into(),
        argv,
        quarantined: quarantine::check_held(&planned.config, root, id).map(|q| q.target),
    })
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
    let last = record::last_done(&text, &tree);
    let why = match record::state(last.as_ref()) {
        State::Passed => match last.as_ref().and_then(left_elsewhere) {
            // Once per tree, whatever `on_stop` asks: the steps passed.
            Some(left) if !asked_before(&text, &tree, session) => left,
            _ => return None,
        },
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

/// For a pass that left steps to where an unmet need is proved, what to
/// tell the person: the change isn't proved until that place has run them.
fn left_elsewhere(event: &serde_json::Value) -> Option<String> {
    let unmet = event["unmet"].as_array().filter(|u| !u.is_empty())?;
    let mut needs: Vec<&str> = unmet.iter().filter_map(|u| u["need"].as_str()).collect();
    needs.dedup();
    Some(format!(
        "the last `fairlead done` passed, but {} step(s) weren't run here because need {} isn't met on this machine. Say so in the pull request and keep it a draft until they pass where the need says they're proved; then stop.",
        unmet.len(),
        needs
            .iter()
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ))
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
