//! `fairlead resume`: where the last session on this branch stopped, for a
//! new one. The last brief, what changed since its base, the done gate for
//! the tree as it is, `next`, and the lessons the branch added. The
//! SessionStart hook runs it, so a session starts with it in context.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::plan::Status;
use fairlead_tests::git;
use serde::Serialize;

use crate::brief_cmd::{self, Brief, Store};
use crate::knowledge::lesson;
use crate::receipt_cmd::{self, Gate};

/// Paths, outside files and lessons each list this many before "N more".
const SHOWN: usize = 5;
const NO_BRIEF: &str = "resume: no brief on this branch yet; `fairlead brief <paths>` lists what a change reaches and starts one";

#[derive(clap::Args)]
pub struct ResumeArgs {
    /// The agent session whose brief to resume, over `CLAUDE_CODE_SESSION_ID`;
    /// without one, or with no brief of its own, the newest brief on this branch.
    #[arg(long)]
    session: Option<String>,
    /// Print it as JSON.
    #[arg(long, conflicts_with = "hook")]
    json: bool,
    /// Read a SessionStart hook's JSON on stdin and answer in its shape;
    /// silent when there's no brief or anything fails.
    #[arg(long)]
    hook: bool,
}

#[derive(Serialize)]
pub struct Resume {
    pub brief: String,
    /// `session` when the session's own brief was found, `branch` for the newest on the branch.
    pub found_by: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub updated_at: String,
    pub paths: Vec<String>,
    /// Changed files since the brief's base, untracked ones included, that the brief named.
    pub in_brief: Vec<String>,
    /// Changed files the brief didn't name.
    pub outside: Vec<String>,
    pub gate: Gate,
    pub next: String,
    /// Lessons added since the merge base.
    pub lessons: Vec<Added>,
}

#[derive(Serialize)]
pub struct Added {
    pub id: String,
    pub title: String,
}

pub fn run(args: ResumeArgs, cwd: &Path) -> ExitCode {
    if args.hook {
        return hook();
    }
    let session = args
        .session
        .clone()
        .or_else(|| std::env::var(brief_cmd::SESSION_ENV).ok());
    match resume(cwd, session.as_deref()) {
        Ok(Some(r)) if args.json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&r).expect("resume serializes")
            );
            ExitCode::SUCCESS
        }
        Ok(Some(r)) => {
            print!("{}", text(&r));
            ExitCode::SUCCESS
        }
        Ok(None) if args.json => {
            println!("{}", serde_json::json!({ "brief": null }));
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("{NO_BRIEF}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("resume: {e}");
            ExitCode::from(2)
        }
    }
}

/// The SessionStart hook: the resume text as context for the new session.
fn hook() -> ExitCode {
    let mut input = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut input);
    if let Some(context) = hook_context(&input) {
        let answer = serde_json::json!({
            "hookSpecificOutput": { "hookEventName": "SessionStart", "additionalContext": context }
        });
        println!("{answer}");
    }
    ExitCode::SUCCESS
}

fn hook_context(input: &str) -> Option<String> {
    let call: serde_json::Value = serde_json::from_str(input).unwrap_or_default();
    let dir = call["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    let r = resume(&dir, call["session_id"].as_str()).ok()??;
    Some(format!(
        "fairlead: where the last session on this branch stopped.\n{}",
        text(&r)
    ))
}

/// The brief to resume: the session's own, else the newest on this branch.
fn find(root: &Path, session: Option<&str>) -> Option<(Brief, &'static str)> {
    let store = Store::open(root)?;
    if let Some(b) = session.and_then(|s| store.current(Some(s))) {
        return Some((b, "session"));
    }
    let base = brief_cmd::base(root, None);
    store
        .newest_on(git::branch(root).as_deref(), base.as_deref())
        .map(|b| (b, "branch"))
}

pub fn resume(cwd: &Path, session: Option<&str>) -> Result<Option<Resume>, String> {
    let root = crate::graph_cmd::repo_root(cwd);
    let Some((brief, found_by)) = find(&root, session) else {
        return Ok(None);
    };
    let s = receipt_cmd::state_for(cwd, Some(brief.clone()), brief.base.clone())?;
    let (in_brief, outside): (Vec<String>, Vec<String>) = s
        .changed
        .iter()
        .cloned()
        .partition(|p| brief.paths.contains(p));
    let dir = format!("{}/", s.now.config.memory.dir.trim_end_matches('/'));
    let (known, _) = lesson::load(&root, &s.now.config.memory);
    let lessons = s
        .now
        .plan
        .changed
        .iter()
        .filter(|c| c.status == Status::Added && c.path.starts_with(&dir))
        .filter_map(|c| {
            let id = Path::new(&c.path).file_stem()?.to_str()?.to_string();
            let title = known
                .iter()
                .find(|l| l.front.id == id)
                .map_or(String::new(), |l| l.front.title.clone());
            Some(Added { id, title })
        })
        .collect();
    Ok(Some(Resume {
        next: receipt_cmd::next_line(&s),
        gate: s.gate,
        brief: brief.id,
        found_by,
        session: brief.session,
        branch: brief.branch,
        base: brief.base,
        updated_at: brief.updated_at,
        paths: brief.paths,
        in_brief,
        outside,
        lessons,
    }))
}

/// A list's first few names and how many more, on one line.
fn names(items: &[String]) -> String {
    let shown: Vec<&str> = items.iter().take(SHOWN).map(String::as_str).collect();
    let more = items.len().saturating_sub(SHOWN);
    if more > 0 {
        format!("{} … {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// At most 20 lines: the brief, what changed, the gate, the lessons, next.
pub fn text(r: &Resume) -> String {
    let base = r
        .base
        .as_deref()
        .map_or("none".into(), |c| c.chars().take(7).collect::<String>());
    let whose = match r.found_by {
        "session" => "this session's",
        _ => "the newest on this branch",
    };
    let at = r
        .updated_at
        .get(..16)
        .unwrap_or(&r.updated_at)
        .replace('T', " ");
    let mut out = format!(
        "resume   brief {}  base {base}  updated {at}  ({whose})\n",
        r.brief
    );
    out.push_str(&format!("  paths  {}\n", names(&r.paths)));
    let total = r.in_brief.len() + r.outside.len();
    out.push_str(&format!(
        "changed  {total} file{} since its base: {} in the brief, {} outside\n",
        if total == 1 { "" } else { "s" },
        r.in_brief.len(),
        r.outside.len()
    ));
    if !r.outside.is_empty() {
        out.push_str(&format!("  outside {}\n", names(&r.outside)));
    }
    out.push_str(&format!("gate     {}\n", receipt_cmd::gate_line(&r.gate)));
    match r.lessons.len() {
        0 => out.push_str("lessons  none added on this branch\n"),
        n => {
            out.push_str(&format!(
                "lessons  {n} added on this branch (fairlead lessons list)\n"
            ));
            for l in r.lessons.iter().take(SHOWN) {
                out.push_str(&format!("  {:<40} {}\n", l.id, l.title));
            }
            if n > SHOWN {
                out.push_str(&format!("  … {} more\n", n - SHOWN));
            }
        }
    }
    out.push_str(&r.next.replacen("next: ", "next     ", 1));
    out.push('\n');
    out
}
