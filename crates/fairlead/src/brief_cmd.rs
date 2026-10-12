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
    /// The branch HEAD was on, so `resume` can find the brief from a new session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The task the branch names, from `[tracker]`: outside text, cleaned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<crate::tracker::Task>,
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
    /// How many skills the text lists before "N more".
    #[serde(default = "skill_cap")]
    pub skill_cap: usize,
}

fn skill_cap() -> usize {
    8
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
    let (made, _) = build(
        cwd,
        &args.paths,
        args.base.as_deref(),
        args.session.clone(),
        &args.sets,
    )?;
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

/// The brief for `paths`, kept in the session's store and offered as an
/// event, with the plan it was made from: what `brief` and `context` share.
pub fn build(
    cwd: &Path,
    named: &[String],
    base_arg: Option<&str>,
    session: Option<String>,
    sets: &[String],
) -> Result<(Brief, Planned), String> {
    let root = crate::graph_cmd::repo_root(cwd);
    let session = session.or_else(|| std::env::var(SESSION_ENV).ok());
    let store = Store::open(&root);
    let base = base(&root, base_arg);
    let mut paths = named.to_vec();
    let loaded = fairlead_core::config::load(cwd, &LoadOptions::from_process(sets.to_vec()))
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
    let planned = make(cwd, &Changes::new(None, paths, sets.to_vec()))?;
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
    let mut made = assemble(&planned, &root, id, session, base, created, now)?;
    made.branch = git::branch(&root);
    // A task is read once per brief: a tracker call can take seconds.
    made.task = match earlier.and_then(|b| b.task) {
        Some(task) => Some(task),
        None => crate::tracker::for_branch(&root, &loaded.config.tracker, made.branch.as_deref())
            .unwrap_or_else(|e| {
                made.warnings.push(format!("tracker: {e}"));
                None
            }),
    };
    if let Some(store) = &store {
        store.save(&made)?;
    }
    offer(&root, &made);
    Ok((made, planned))
}

/// One `offer` event naming what the brief offered, for the hit rate:
/// `skills report` sets it against what the agent then used.
fn offer(root: &Path, brief: &Brief) {
    let items: Vec<String> = brief
        .lessons
        .items
        .iter()
        .map(|i| format!("lesson:{}", i.name))
        .chain(
            brief
                .skills
                .items
                .iter()
                .map(|i| format!("skill:{}", i.name)),
        )
        .collect();
    if items.is_empty() {
        return;
    }
    if let Some(log) = EventLog::open(root) {
        let mut event = Event::new("offer", "brief", std::time::Duration::ZERO);
        event.session = brief.session.clone();
        event.items = items;
        let _ = log.append(&event);
    }
}

/// The merge base with `base`, or with the default branch.
pub fn base(root: &Path, base: Option<&str>) -> Option<String> {
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
    let reach = crate::knowledge::reach(planned);
    let offered = crate::knowledge::lessons(planned, root, &reach);
    let routed = crate::knowledge::skills(planned, root, &reach);
    Ok(Brief {
        version: 1,
        id,
        session,
        created_at,
        updated_at,
        base,
        branch: None,
        task: None,
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
            source: "fairlead.toml [[skills.routes]]".into(),
            items: routed
                .lessons
                .iter()
                .map(|(n, w)| item(n.clone(), w.clone()))
                .collect(),
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
            .chain(
                routed
                    .bad
                    .iter()
                    .map(|b| format!("[bad-skill] {}: {}", b.path, b.reason)),
            )
            .collect(),
        lesson_cap: planned.config.memory.cap,
        skill_cap: planned.config.skills.cap,
    })
}

/// The brief as an agent reads it: a line per section with its source on
/// the right, and the first few items of each unless `all`.
/// The task row: the tracker's words, quoted as text from outside.
fn task_lines(t: &crate::tracker::Task) -> String {
    if t.source == "agent" {
        return format!(
            "task     {}  look it up with your tracker tool; Fairlead doesn't read it\n",
            t.id
        );
    }
    let mut out = format!("task     {}  \"{}\"", t.id, t.title);
    if let Some(status) = &t.status {
        out.push_str(&format!("  [{status}]"));
    }
    if let Some(url) = &t.url {
        out.push_str(&format!("  {url}"));
    }
    out.push('\n');
    if let Some(done) = &t.done_when {
        out.push_str(&format!("  done when (from the tracker): {done}\n"));
    }
    out
}

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
    if let Some(t) = &b.task {
        out.push_str(&task_lines(t));
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
    let n = b.skills.items.len();
    let skills = match n {
        0 => "none".to_string(),
        1 => "1 skill".to_string(),
        n => format!("{n} skills"),
    };
    head(&mut out, "skills", skills, &b.skills.source);
    capped(
        &mut out,
        &b.skills.items,
        "fairlead brief --all",
        b.skill_cap,
    );
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

    /// The newest brief of any session made on `branch`; a brief that
    /// recorded no branch counts when it has the same base.
    pub fn newest_on(&self, branch: Option<&str>, base: Option<&str>) -> Option<Brief> {
        std::fs::read_dir(&self.dir)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Brief>(&t).ok())
            .filter(|b| match &b.branch {
                Some(recorded) => Some(recorded.as_str()) == branch,
                None => b.base.is_some() && b.base.as_deref() == base,
            })
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

/// The PostToolUse hook. After an edit: once per session with no brief, a
/// note saying how to get one, and once per skill per session, the routed
/// skill that applies to the file. After a skill is loaded (Claude Code's
/// `Skill`, or a read of a SKILL.md), a `use` event for the hit rate. Never
/// a deny, and silent on any error.
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

const READS: [&str; 3] = ["Read", "read_file", "read_many_files"];

fn nudge_for(call: &serde_json::Value) -> Option<String> {
    let session = call["session_id"].as_str()?;
    let dir = call["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    let loaded = fairlead_core::config::load(&dir, &LoadOptions::from_process(Vec::new())).ok()?;
    let root = crate::graph_cmd::repo_root(&dir);
    let log = EventLog::open(&root)?;
    let tool = call["tool_name"].as_str().unwrap_or("");
    let input = &call["tool_input"];
    let file = input["file_path"]
        .as_str()
        .or_else(|| input["absolute_path"].as_str())
        .or_else(|| input["path"].as_str());
    let (skills, _) = crate::knowledge::skill::load(&root, &loaded.config.skills);
    let record = |stage: &'static str, decision: &'static str, items: Vec<String>| {
        let mut event = Event::new(stage, decision, std::time::Duration::ZERO);
        event.session = Some(session.to_string());
        event.tool = Some(tool.to_string());
        event.items = items;
        let _ = log.append(&event);
    };
    if tool == "Skill" {
        let name = input["skill"].as_str().or_else(|| input["name"].as_str())?;
        let name = name.rsplit(':').next().unwrap_or(name);
        record("use", "skill", vec![format!("skill:{name}")]);
        return None;
    }
    if READS.contains(&tool) {
        let used = file.and_then(|f| crate::knowledge::skill::by_path(&skills, f))?;
        record("use", "read", vec![format!("skill:{}", used.name)]);
        return None;
    }
    let events: Vec<serde_json::Value> = log
        .read()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|e: &serde_json::Value| e["session"] == session)
        .collect();
    let mut notes = Vec::new();
    let has_brief = Store::open(&root)?.current(Some(session)).is_some();
    if loaded.config.brief.nudge && !has_brief && !events.iter().any(|e| e["stage"] == "nudge") {
        record("nudge", "note", Vec::new());
        notes.push(NUDGE.to_string());
    }
    if let Some(file) = file {
        let rel = Path::new(file)
            .strip_prefix(std::fs::canonicalize(&root).unwrap_or(root.clone()))
            .or_else(|_| Path::new(file).strip_prefix(&root))
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| file.to_string());
        let told: Vec<&str> = events
            .iter()
            .filter(|e| e["stage"] == "offer" && e["decision"] == "nudge")
            .flat_map(|e| e["items"].as_array().into_iter().flatten())
            .filter_map(|i| i.as_str())
            .collect();
        for s in skills
            .iter()
            .filter(|s| s.scope.paths.iter().any(|p| p.is_match(&rel)))
        {
            let item = format!("skill:{}", s.name);
            if told.contains(&item.as_str()) {
                continue;
            }
            record("offer", "nudge", vec![item]);
            let about = if s.description.is_empty() {
                String::new()
            } else {
                format!(": {}", s.description)
            };
            notes.push(format!(
                "fairlead: the `{}` skill applies to {rel}{about} Load it before editing further ({}).",
                s.name, s.path
            ));
        }
    }
    (!notes.is_empty()).then(|| notes.join("\n"))
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
