//! `fairlead guard hook`: the write stage, run by Claude Code or Codex before
//! an edit or a shell command. It answers only with a deny or a note, and anything
//! it can't decide in time, read or understand lets the call go ahead.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fairlead_core::config::{self, LoadOptions};
use fairlead_guard::edit::Rebuilt;
use fairlead_guard::events::{Event, EventLog};
use fairlead_guard::hook::{self, Request};
use fairlead_guard::patch::{self, Change, FilePatch};
use fairlead_guard::{added, git, head_paths, Finding, Guard, Source};
use serde_json::Value;

/// A file larger than this is let through unread: a minified bundle would
/// cost more than the budget to parse.
const MAX_BYTES: usize = 256 * 1024;

/// What the check decided, for the answer and the event log.
struct Outcome {
    decision: &'static str,
    answer: Option<Value>,
    file: Option<String>,
    found: Vec<Finding>,
}

impl Outcome {
    fn allow(file: Option<String>) -> Outcome {
        Outcome {
            decision: "allow",
            answer: None,
            file,
            found: Vec::new(),
        }
    }
}

pub fn run() -> ExitCode {
    let start = Instant::now();
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return ExitCode::SUCCESS;
    }
    let Ok(call) = serde_json::from_str::<Value>(&input) else {
        return ExitCode::SUCCESS;
    };
    let dir = call
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let Ok(loaded) = config::load(&dir, &LoadOptions::from_process(Vec::new())) else {
        return ExitCode::SUCCESS;
    };
    let settings = loaded.config.guard.clone();
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(&dir)
    } else {
        loaded.root.clone()
    };
    let log = match settings.events {
        config::Events::Local => EventLog::open(&root),
        config::Events::Off => None,
    };
    let tool = call
        .get("tool_name")
        .and_then(Value::as_str)
        .map(String::from);
    let record = |decision: &'static str, outcome: Option<&Outcome>| {
        let Some(log) = &log else { return };
        let mut event = Event::new("write", decision, start.elapsed());
        event.event = Some("PreToolUse".into());
        event.tool = tool.clone();
        if let Some(o) = outcome {
            event.file = o.file.clone();
            event.added = o.found.len();
            event.rules = o
                .found
                .iter()
                .map(|f| f.rule)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
        }
        // The log is a record, never a reason to block or fail a write.
        let _ = log.append(&event);
    };
    if !loaded.problems.is_empty() {
        record("error", None);
        return ExitCode::SUCCESS;
    }
    let budget = Duration::from_millis(u64::from(settings.budget_ms));
    let (send, receive) = mpsc::channel();
    let thread_root = root.clone();
    std::thread::spawn(move || {
        let outcome =
            std::panic::catch_unwind(|| decide(&call, &loaded.config.guard, &thread_root, &dir));
        let _ = send.send(outcome);
    });
    match receive.recv_timeout(budget.saturating_sub(start.elapsed())) {
        Ok(Ok(Some(outcome))) => {
            record(outcome.decision, Some(&outcome));
            if let Some(answer) = &outcome.answer {
                println!("{answer}");
            }
        }
        Ok(Ok(None)) => {}
        Ok(Err(_)) => record("error", None),
        Err(_) => record("timed_out", None),
    }
    ExitCode::SUCCESS
}

/// None for a call the guard has no rules for, which isn't logged.
fn decide(call: &Value, settings: &config::Guard, root: &Path, dir: &Path) -> Option<Outcome> {
    let guard = Guard::new(settings, root).ok()?;
    if guard.is_empty() {
        return None;
    }
    let warn = settings.on_finding == config::OnFinding::Warn;
    match hook::request(call) {
        Request::Ignored => None,
        Request::Unknown { .. } => Some(Outcome::allow(None)),
        Request::Bash { command } => {
            let reason = guard.commands.denied(&command)?;
            let text = format!("fairlead guard: {reason}");
            Some(Outcome {
                decision: if warn { "warn" } else { "deny" },
                answer: Some(if warn {
                    hook::warn(&text)
                } else {
                    hook::deny(&text)
                }),
                file: None,
                found: Vec::new(),
            })
        }
        Request::Write { path, text } => Some(check_file(
            &guard,
            settings,
            root,
            dir,
            &path,
            Some(text),
            &[],
        )),
        Request::Edit { path, edits } => {
            Some(check_file(&guard, settings, root, dir, &path, None, &edits))
        }
        Request::Patch { files } => Some(check_patch(&guard, settings, root, dir, &files)),
    }
}

fn check_file(
    guard: &Guard,
    settings: &config::Guard,
    root: &Path,
    dir: &Path,
    path: &str,
    whole: Option<String>,
    edits: &[(String, String, bool)],
) -> Outcome {
    let abs = dir.join(path);
    let Some(rel) = relative(root, &abs) else {
        return Outcome::allow(None);
    };
    if let Some(denied) = migration_denied(guard, settings, root, &rel) {
        return denied;
    }
    if !guard.reads(&rel) {
        return Outcome::allow(Some(rel));
    }
    let Some(current) = read_current(&abs) else {
        return Outcome::allow(Some(rel));
    };
    let after = match whole {
        Some(text) => text,
        None => match hook::rebuild(&current, edits) {
            Rebuilt::Text(text) => text,
            Rebuilt::Unknown(_) => return Outcome::allow(Some(rel)),
        },
    };
    let found = lint_change(guard, settings, &rel, &current, &after);
    answer(settings, rel, found)
}

/// Every file of a Codex patch, checked as a write or an edit would be, in
/// one answer. A file the patch can't be applied to is let through: Codex
/// refuses the whole patch then.
fn check_patch(
    guard: &Guard,
    settings: &config::Guard,
    root: &Path,
    dir: &Path,
    files: &[FilePatch],
) -> Outcome {
    let mut found = Vec::new();
    let mut first = None;
    for file in files {
        let abs = dir.join(&file.path);
        let Some(rel) = relative(root, &abs) else {
            continue;
        };
        // Deleting or moving a migration rewrites history as editing it does.
        if let Some(denied) = migration_denied(guard, settings, root, &rel) {
            return denied;
        }
        let (target, chunks) = match &file.change {
            Change::Delete => continue,
            Change::Add(_) => (rel, None),
            Change::Update { to: None, chunks } => (rel, Some(chunks)),
            Change::Update {
                to: Some(to),
                chunks,
            } => {
                let Some(to) = relative(root, &dir.join(to)) else {
                    continue;
                };
                if let Some(denied) = migration_denied(guard, settings, root, &to) {
                    return denied;
                }
                (to, Some(chunks))
            }
        };
        if !guard.reads(&target) {
            continue;
        }
        let Some(current) = read_current(&abs) else {
            continue;
        };
        let after = match (&file.change, chunks) {
            (Change::Add(text), _) => text.clone(),
            (_, Some(chunks)) => match patch::apply(&current, chunks) {
                Rebuilt::Text(text) => text,
                Rebuilt::Unknown(_) => continue,
            },
            _ => continue,
        };
        let here = lint_change(guard, settings, &target, &current, &after);
        if first.is_none() || (!here.is_empty() && found.is_empty()) {
            first = Some(target);
        }
        found.extend(here);
    }
    match first {
        Some(rel) => answer(settings, rel, found),
        None => Outcome::allow(None),
    }
}

/// A deny, or a warning, for writing to a migration that already exists.
fn migration_denied(
    guard: &Guard,
    settings: &config::Guard,
    root: &Path,
    rel: &str,
) -> Option<Outcome> {
    let m = guard
        .migrations
        .as_ref()
        .filter(|m| m.immutable() && m.covers(rel))?;
    let existed = match m.base().map(|b| git::merge_base(root, b)) {
        Some(Ok(base)) => git::exists_at(root, &base, rel),
        _ => head_paths::exists_at_head(root, &m.dirs(), rel),
    };
    if !existed {
        return None;
    }
    let text = format!(
        "fairlead guard: {rel} is a migration that already exists, and a database that ran it won't run it again. Add a new migration instead."
    );
    let warn = settings.on_finding == config::OnFinding::Warn;
    Some(Outcome {
        decision: if warn { "warn" } else { "deny" },
        answer: Some(if warn {
            hook::warn(&text)
        } else {
            hook::deny(&text)
        }),
        file: Some(rel.to_string()),
        found: Vec::new(),
    })
}

/// The file's text now, empty for a new file; none to let the call through
/// unread.
fn read_current(abs: &Path) -> Option<String> {
    match std::fs::read(abs) {
        Ok(bytes) => String::from_utf8(bytes).ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(String::new()),
        Err(_) => None,
    }
}

fn lint_change(
    guard: &Guard,
    settings: &config::Guard,
    rel: &str,
    current: &str,
    after: &str,
) -> Vec<Finding> {
    if after.len() > MAX_BYTES || current.len() > MAX_BYTES {
        return Vec::new();
    }
    // Before and after are linted at once: each is a full parse, and the
    // budget is wall time.
    match settings.findings {
        config::Findings::All => guard.lint(&Source::new(rel, after)),
        config::Findings::Added => std::thread::scope(|scope| {
            let before = scope.spawn(|| guard.lint(&Source::new(rel, current)));
            let now = guard.lint(&Source::new(rel, after));
            match before.join() {
                Ok(before) => added::added(&before, &now),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }),
    }
}

fn answer(settings: &config::Guard, rel: String, found: Vec<Finding>) -> Outcome {
    if found.is_empty() {
        return Outcome::allow(Some(rel));
    }
    let warn = settings.on_finding == config::OnFinding::Warn;
    let list: Vec<String> = found.iter().map(|f| format!("  {f}")).collect();
    let what = match settings.findings {
        config::Findings::Added => "this edit adds",
        config::Findings::All => "this file would have",
    };
    let answer = if warn {
        hook::warn(&format!(
            "fairlead guard: {what} {} finding(s); CI will fail on them:\n{}",
            found.len(),
            list.join("\n")
        ))
    } else {
        hook::deny(&format!(
            "fairlead guard: {what} {} finding(s):\n{}\nFix them and write again.",
            found.len(),
            list.join("\n")
        ))
    };
    Outcome {
        decision: if warn { "warn" } else { "deny" },
        answer: Some(answer),
        file: Some(rel),
        found,
    }
}

/// `abs` from the project root, with `/`; none for a path outside it. The
/// nearest part of it that exists is resolved, so a new file in a new
/// directory is compared with the root the same way an existing one is.
fn relative(root: &Path, abs: &Path) -> Option<String> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut existing = abs;
    let mut rest = Vec::new();
    let resolved = loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            break real;
        }
        rest.push(existing.file_name()?);
        existing = existing.parent()?;
    };
    let full = rest
        .iter()
        .rev()
        .fold(resolved, |path, part| path.join(part));
    let rel = full.strip_prefix(&root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}
