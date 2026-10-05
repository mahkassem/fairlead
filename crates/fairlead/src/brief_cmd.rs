//! `fairlead brief`: before an edit, what the named paths reach, the tests
//! and checks that will run, the rules that read them and the done gate,
//! each with where it came from. The brief is kept per session so the
//! receipt can compare what changed against what the agent was told.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use fairlead_core::config::{BriefPer, LoadOptions};
use fairlead_guard::events::{timestamp, Event, EventLog};
use fairlead_tests::{git, render};
use serde::{Deserialize, Serialize};

use crate::plan_cmd::{make, Changes, Planned};

/// Each section shows this many items unless `--all` is given.
const SHOWN: usize = 5;
/// The variable Claude Code sets for the commands an agent runs; it equals
/// the `session_id` its hooks receive.
pub const SESSION_ENV: &str = "CLAUDE_CODE_SESSION_ID";
const NUDGE: &str = "fairlead: no brief for this change yet; `fairlead brief <paths>` lists what those paths reach, the tests that will run and the done gate.";

#[derive(clap::Args)]
pub struct BriefArgs {
    /// The files or directories the change will touch; a path that doesn't exist yet is a new file.
    #[arg(required = true, num_args = 1..)]
    paths: Vec<String>,
    /// The branch or commit the change starts from; its merge base with HEAD is used.
    #[arg(long)]
    base: Option<String>,
    /// The agent session the brief belongs to, over `CLAUDE_CODE_SESSION_ID`.
    #[arg(long)]
    session: Option<String>,
    /// List every item instead of the first few of each section.
    #[arg(long)]
    all: bool,
    /// Print the brief as JSON.
    #[arg(long)]
    json: bool,
    /// Override a config value for this run.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    sets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brief {
    pub version: u32,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// The commit the change is measured from, when git has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub paths: Vec<String>,
    /// Named paths that aren't in the tree yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new: Vec<String>,
    pub plan_id: String,
    pub reaches: Reaches,
    pub tests: Section<Item>,
    pub checks: Section<Item>,
    pub rules: Section<Rule>,
    pub lessons: Section<Item>,
    pub skills: Section<Item>,
    pub done: Section<Item>,
    /// Inputs the brief couldn't use, such as a lesson file it couldn't read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// How many lessons the text lists before "N more".
    #[serde(default = "lesson_cap")]
    pub lesson_cap: usize,
}

fn lesson_cap() -> usize {
    SHOWN
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section<T> {
    /// Where the section's facts come from.
    pub source: String,
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub name: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// The `[guard.*]` table.
    pub table: String,
    /// The named paths it reads.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaches {
    pub source: String,
    /// Files that import a named path themselves.
    pub direct: Vec<Item>,
    /// Every file that depends on a named path, through any chain.
    pub total: usize,
}

pub fn run(args: BriefArgs, cwd: &Path) -> ExitCode {
    match brief(&args, cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("brief: {e}");
            ExitCode::from(2)
        }
    }
}

fn brief(args: &BriefArgs, cwd: &Path) -> Result<(), String> {
    let root = crate::graph_cmd::repo_root(cwd);
    let session = args
        .session
        .clone()
        .or_else(|| std::env::var(SESSION_ENV).ok());
    let store = Store::open(&root);
    let base = base(&root, args.base.as_deref());
    let mut paths = args.paths.clone();
    let loaded = fairlead_core::config::load(cwd, &LoadOptions::from_process(args.sets.clone()))
        .map_err(|e| e.to_string())?;
    let per = loaded.config.brief.per;
    let earlier = match (&store, per) {
        (Some(store), BriefPer::Session) => {
            store.current(session.as_deref()).filter(|b| b.base == base)
        }
        _ => None,
    };
    if let Some(earlier) = &earlier {
        // Stored paths are from the root; absolute, they resolve from any directory.
        let root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        paths.extend(
            earlier
                .paths
                .iter()
                .map(|p| root.join(p).display().to_string()),
        );
    }
    let planned = make(cwd, &Changes::new(None, paths, args.sets.clone()))?;
    let now = timestamp(SystemTime::now());
    let id = match (&earlier, per) {
        (Some(b), _) => b.id.clone(),
        (None, BriefPer::Session) => {
            format!("b-{:08x}", fnv(session.as_deref().unwrap_or("")) as u32)
        }
        (None, BriefPer::Call) => format!("b-{:08x}", fnv(&format!("{session:?}{now}")) as u32),
    };
    let created = earlier
        .as_ref()
        .map_or(now.clone(), |b| b.created_at.clone());
    let made = assemble(&planned, &root, id, session, base, created, now)?;
    if let Some(store) = &store {
        store.save(&made)?;
    }
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&made).expect("brief serializes")
        );
    } else {
        print!("{}", text(&made, args.all));
    }
    Ok(())
}

/// The merge base with `base`, or with the default branch.
fn base(root: &Path, base: Option<&str>) -> Option<String> {
    let base = match base {
        Some(b) => b.to_string(),
        None => git::default_base(root).ok()?,
    };
    git::merge_base(root, &base).ok()
}

fn assemble(
    planned: &Planned,
    root: &Path,
    id: String,
    session: Option<String>,
    base: Option<String>,
    created_at: String,
    updated_at: String,
) -> Result<Brief, String> {
    let plan = &planned.plan;
    let graph = &planned.scan.graph;
    let paths: Vec<String> = plan.changed.iter().map(|c| c.path.clone()).collect();
    let ids: Vec<u32> = paths.iter().filter_map(|p| graph.id(p)).collect();
    let named: BTreeSet<u32> = ids.iter().copied().collect();
    let mut direct: BTreeMap<String, String> = BTreeMap::new();
    for &id in &ids {
        for (importer, kind) in graph.importers(id) {
            if !named.contains(&importer) {
                let kind = kind.map_or("package".into(), |k| format!("{k:?}").to_lowercase());
                direct
                    .entry(graph.files[importer as usize].clone())
                    .or_insert(format!("imports {} ({kind})", graph.files[id as usize]));
            }
        }
    }
    let total = graph.affected(&ids).len() - named.len();
    let new = paths
        .iter()
        .filter(|p| !planned.scan.tree.contains(p))
        .cloned()
        .collect();
    let guard = fairlead_guard::Guard::new(&planned.config.guard, root)?;
    let mut tables: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for p in &paths {
        for table in guard.tables_for(p) {
            tables.entry(table).or_default().push(p.clone());
        }
    }
    let item = |name: String, why: String| Item { name, why };
    let held = |target: &str| {
        plan.quarantined
            .iter()
            .find(|q| q.target == target)
            .map_or(String::new(), |q| {
                format!("; not provable here [{}]", q.here.join(", "))
            })
    };
    let steps = crate::done_cmd::steps(planned)?;
    let reach = crate::knowledge::reach(planned, 1);
    let offered = crate::knowledge::lessons(planned, root, &reach);
    Ok(Brief {
        version: 1,
        id,
        session,
        created_at,
        updated_at,
        base,
        paths,
        new,
        plan_id: plan.plan_id.clone(),
        reaches: Reaches {
            source: "graph importers, graph why".into(),
            direct: direct.into_iter().map(|(n, w)| item(n, w)).collect(),
            total,
        },
        tests: Section {
            source: "plan --files".into(),
            items: plan
                .tests
                .iter()
                .map(|t| item(t.path.clone(), render::reason(&t.reason) + &held(&t.path)))
                .collect(),
        },
        checks: Section {
            source: "plan --files".into(),
            items: plan
                .checks
                .iter()
                .map(|c| item(c.id.clone(), render::reason(&c.reason) + &held(&c.id)))
                .collect(),
        },
        rules: Section {
            source: "fairlead.toml [guard.*]".into(),
            items: tables
                .into_iter()
                .map(|(t, p)| Rule {
                    table: t.to_string(),
                    paths: p,
                })
                .collect(),
        },
        lessons: Section {
            source: format!("{} (fairlead lessons list)", planned.config.memory.dir),
            items: offered
                .lessons
                .into_iter()
                .map(|(n, w)| item(n, w))
                .collect(),
        },
        skills: Section {
            source: "none routed yet (K4)".into(),
            items: Vec::new(),
        },
        done: Section {
            source: "fairlead.toml [done], done --dry-run".into(),
            items: steps
                .iter()
                .map(|s| item(s.id.clone(), s.argv.join(" ")))
                .collect(),
        },
        warnings: offered
            .bad
            .iter()
            .map(|b| format!("[bad-lesson] {}: {}", b.path, b.reason))
            .collect(),
        lesson_cap: planned.config.memory.cap,
    })
}

/// The brief as an agent reads it: a line per section with its source on
/// the right, and the first few items of each unless `all`.
pub fn text(b: &Brief, all: bool) -> String {
    let mut out = String::new();
    let base = b
        .base
        .as_deref()
        .map_or("none".into(), |c| c.chars().take(7).collect::<String>());
    out.push_str(&format!(
        "brief {}  base {base}  {} path{}\n",
        b.id,
        b.paths.len(),
        if b.paths.len() == 1 { "" } else { "s" }
    ));
    for p in &b.new {
        out.push_str(&format!(
            "  new      {p}: not in the tree yet, so nothing reaches it\n"
        ));
    }
    let head = |out: &mut String, name: &str, summary: String, source: &str| {
        out.push_str(&format!("{name:<8} {summary:<56} {source}\n"));
    };
    let capped = |out: &mut String, items: &[Item], more: &str, cap: usize| {
        let shown = if all {
            items.len()
        } else {
            items.len().min(cap)
        };
        for i in &items[..shown] {
            out.push_str(&format!("  {:<44} {}\n", i.name, i.why));
        }
        if shown < items.len() {
            out.push_str(&format!("  … {} more ({more})\n", items.len() - shown));
        }
    };
    let list = |out: &mut String, items: &[Item], more: &str| capped(out, items, more, SHOWN);
    head(
        &mut out,
        "reaches",
        format!(
            "{} {} directly, {} through the graph",
            files(b.reaches.direct.len()),
            if b.reaches.direct.len() == 1 {
                "imports them"
            } else {
                "import them"
            },
            b.reaches.total
        ),
        &b.reaches.source,
    );
    list(&mut out, &b.reaches.direct, "fairlead brief --all");
    head(
        &mut out,
        "tests",
        format!(
            "{} test {}",
            b.tests.items.len(),
            if b.tests.items.len() == 1 {
                "file"
            } else {
                "files"
            }
        ),
        &b.tests.source,
    );
    list(&mut out, &b.tests.items, "fairlead test --explain FILE");
    let ids = |items: &[Item]| {
        items
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let or_none = |s: String| if s.is_empty() { "none".to_string() } else { s };
    head(
        &mut out,
        "checks",
        or_none(ids(&b.checks.items)),
        &b.checks.source,
    );
    let rules: Vec<&str> = b.rules.items.iter().map(|r| r.table.as_str()).collect();
    head(
        &mut out,
        "rules",
        or_none(rules.join(", ")),
        &b.rules.source,
    );
    let n = b.lessons.items.len();
    let lessons = match n {
        0 => "none".to_string(),
        1 => "1 lesson".to_string(),
        n => format!("{n} lessons"),
    };
    head(&mut out, "lessons", lessons, &b.lessons.source);
    capped(
        &mut out,
        &b.lessons.items,
        "fairlead brief --all",
        b.lesson_cap,
    );
    head(&mut out, "skills", "none".into(), &b.skills.source);
    head(
        &mut out,
        "done",
        or_none(ids(&b.done.items)),
        &b.done.source,
    );
    for w in &b.warnings {
        out.push_str(&format!("warning  {w}\n"));
    }
    out.push_str("next     edit, then: fairlead done\n");
    out
}

/// Briefs in the git directory, one JSON file each, never committed.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Option<Store> {
        fairlead_guard::git::git_dir(root).map(|d| Store {
            dir: d.join("fairlead").join("briefs"),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn save(&self, brief: &Brief) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let path = self.dir.join(format!("{}.json", brief.id));
        let text = serde_json::to_string_pretty(brief).expect("brief serializes") + "\n";
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The newest brief of `session`, or of no session when there's none.
    pub fn current(&self, session: Option<&str>) -> Option<Brief> {
        std::fs::read_dir(&self.dir)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Brief>(&t).ok())
            .filter(|b| b.session.as_deref() == session)
            .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
    }
}

fn files(n: usize) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}

/// FNV-1a, for short stable ids without another dependency.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The Claude Code PostToolUse hook: after the first edit of a session with
/// no brief, one note saying how to get one. Never a deny, and silent on
/// any error.
pub fn nudge() -> ExitCode {
    let mut input = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut input);
    let call: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    if let Some(note) = nudge_for(&call) {
        let answer = serde_json::json!({
            "hookSpecificOutput": { "hookEventName": "PostToolUse", "additionalContext": note }
        });
        println!("{answer}");
    }
    ExitCode::SUCCESS
}

fn nudge_for(call: &serde_json::Value) -> Option<&'static str> {
    let session = call["session_id"].as_str()?;
    let dir = call["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    let loaded = fairlead_core::config::load(&dir, &LoadOptions::from_process(Vec::new())).ok()?;
    if !loaded.config.brief.nudge {
        return None;
    }
    let root = crate::graph_cmd::repo_root(&dir);
    if Store::open(&root)?.current(Some(session)).is_some() {
        return None;
    }
    let log = EventLog::open(&root)?;
    let told = log
        .read()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .any(|e| e["stage"] == "nudge" && e["session"] == session);
    if told {
        return None;
    }
    let mut event = Event::new("nudge", "note", std::time::Duration::ZERO);
    event.session = Some(session.to_string());
    let _ = log.append(&event);
    Some(NUDGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_differ_by_session() {
        assert_eq!(fnv("s1"), fnv("s1"));
        assert_ne!(fnv("s1"), fnv("s2"));
    }
}
