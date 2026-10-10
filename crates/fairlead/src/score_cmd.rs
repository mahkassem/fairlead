//! `fairlead doctor --score`: how ready a repository is, as points on a
//! fixed rubric. Each item asks for evidence that the part works, not only
//! that it's configured, so one line of config can't buy its points, and the
//! rubric is versioned so a score before and after a change compare.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use fairlead_core::config::{self, LoadOptions, Loaded};
use serde::Serialize;
use serde_json::Value;

use crate::knowledge::lesson::{self, days};
use crate::{graph_cmd, hooks_cmd};

pub const RUBRIC: u32 = 1;
/// Where `--replay` looks when not given a report.
pub const REPLAY_REPORT: &str = ".fairlead/replay.json";
const FRESH_DAYS: i64 = 30;
const FIRING_DAYS: i64 = 7;
const RECALL_BAR: f64 = 0.95;

#[derive(Args, Clone, Default)]
pub struct DoctorArgs {
    /// Score readiness on the rubric instead of describing the setup.
    #[arg(long)]
    pub score: bool,
    /// Print the score as JSON.
    #[arg(long, requires = "score")]
    pub json: bool,
    /// Exit 1 when the score is below this, for CI.
    #[arg(long, value_name = "N", requires = "score")]
    pub min: Option<u32>,
    /// A `replay run --json-out` report; defaults to .fairlead/replay.json.
    #[arg(long, value_name = "FILE", requires = "score")]
    pub replay: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct Item {
    pub id: &'static str,
    pub points: u32,
    pub max: u32,
    /// What was found, in a few words.
    pub evidence: String,
    /// The one thing that earns the missing points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Score {
    pub rubric: u32,
    pub score: u32,
    pub items: Vec<Item>,
}

fn item(id: &'static str, max: u32, earned: u32, evidence: String, fix: &str) -> Item {
    Item {
        id,
        points: earned.min(max),
        max,
        evidence,
        fix: (earned < max).then(|| fix.to_string()),
    }
}

fn pass(id: &'static str, max: u32, ok: bool, evidence: String, fix: &str) -> Item {
    item(id, max, if ok { max } else { 0 }, evidence, fix)
}

/// Days from `date` (the first ten characters of a timestamp) to `today`.
fn age(date: &str, today: i64) -> Option<i64> {
    days(date.get(..10)?).map(|d| today - d)
}

/// The newest event at `stage` with `decision`, as its age in days.
fn newest(events: &[Value], stage: &str, decision: &str, today: i64) -> Option<i64> {
    events
        .iter()
        .filter(|e| e["stage"] == stage && e["decision"] == decision)
        .filter_map(|e| age(e["at"].as_str()?, today))
        .min()
}

fn config_item(loaded: &Result<Loaded, config::ConfigError>) -> Item {
    let (ok, evidence) = match loaded {
        Ok(l) if l.files.is_empty() => (false, "no fairlead.toml; defaults apply".to_string()),
        Ok(l) if l.problems.is_empty() => (true, format!("{}, no problems", name(&l.files[0]))),
        Ok(l) => (false, format!("{} problems", l.problems.len())),
        Err(e) => (false, format!("invalid: {e}")),
    };
    pass(
        "config",
        10,
        ok,
        evidence,
        "run `fairlead init`, then fix what `fairlead config check` reports",
    )
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn runners_item(root: &Path, loaded: &Loaded) -> Item {
    let runners = loaded.config.tests.runners.items().len();
    let tree = fairlead_lang::tree::Tree::scan(root);
    let problems = fairlead_tests::testfiles::runner_problems(&tree, &loaded.config)
        .unwrap_or_else(|e| vec![e]);
    let evidence = match (runners, problems.len()) {
        (0, _) => "no test runner".to_string(),
        (n, 0) => format!("{n} runner(s), every test file has exactly one"),
        (_, p) => format!("{p} test file(s) without exactly one runner"),
    };
    pass(
        "runners",
        15,
        runners > 0 && problems.is_empty(),
        evidence,
        "give every test file exactly one runner; `fairlead config check` lists them",
    )
}

/// Agents the repository shows, each scored by whether its hooks are in.
fn hooks_item(root: &Path, loaded: &Loaded) -> Item {
    let lines = hooks_cmd::describe(root, &loaded.config);
    let installed = |agent: &str| {
        lines
            .iter()
            .any(|l| l.starts_with(&format!("{agent} hook: installed")))
    };
    let mut agents: Vec<&str> = [("codex", ".codex"), ("gemini", ".gemini")]
        .into_iter()
        .filter(|(_, dir)| root.join(dir).is_dir())
        .map(|(a, _)| a)
        .collect();
    agents.insert(0, "claude");
    let done: Vec<&str> = agents.iter().copied().filter(|a| installed(a)).collect();
    let earned = (10 * done.len() / agents.len()) as u32;
    item(
        "hooks",
        10,
        earned,
        format!(
            "{} of {} agents: {}",
            done.len(),
            agents.len(),
            agents.join(", ")
        ),
        "run `fairlead hooks install` (with --codex or --gemini for those agents)",
    )
}

fn done_item(events: &[Value], today: i64) -> Item {
    let last = newest(events, "done", "pass", today);
    pass(
        "done",
        10,
        last.is_some_and(|d| d <= FRESH_DAYS),
        match last {
            Some(d) => format!("last passed {d} day(s) ago"),
            None => "no passing `fairlead done` recorded".to_string(),
        },
        "run `fairlead done` and let it pass",
    )
}

fn ci_item(root: &Path, loaded: &Loaded) -> Item {
    let planned = std::fs::read_dir(root.join(".github/workflows"))
        .into_iter()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.ok()?.path()).ok())
        .any(|text| {
            text.contains(crate::ci_workflow::MARKER)
                || text.contains("fairlead ci ")
                || text.contains("mahkassem/fairlead@")
        });
    let staged = loaded.config.stages.is_some();
    let earned = u32::from(planned) * 5 + u32::from(planned && staged) * 5;
    item(
        "ci",
        10,
        earned,
        match (planned, staged) {
            (false, _) => "no workflow runs `fairlead ci`".to_string(),
            (true, false) => "a workflow runs `fairlead ci`, without stages".to_string(),
            (true, true) => "a workflow runs `fairlead ci` with stages".to_string(),
        },
        "run `fairlead ci workflow --write`, with `[stages]` in the config",
    )
}

/// The two replay items: a fresh dataset that met its gate, and recall
/// paired with selection, so a plan that runs everything can't score.
fn replay_items(report: Option<&Value>, today: i64) -> [Item; 2] {
    let fix_fresh =
        "run `fairlead replay fetch` and `fairlead replay run --json-out .fairlead/replay.json`";
    let Some(r) = report else {
        let none = || "no replay report".to_string();
        return [
            pass("replay", 15, false, none(), fix_fresh),
            pass("recall", 15, false, none(), fix_fresh),
        ];
    };
    let judged = r["judged"].as_u64().unwrap_or(0);
    let gate = r["min_failures"].as_u64().unwrap_or(u64::MAX);
    let old = r["until"].as_str().and_then(|u| age(u, today));
    let fresh = old.is_some_and(|d| d <= FRESH_DAYS) && judged >= gate;
    let recall = r["recall"].as_f64();
    let median = r["median_selected"].as_f64();
    let pct = |v: Option<f64>| v.map_or("-".to_string(), |v| format!("{:.1}%", v * 100.0));
    [
        pass(
            "replay",
            15,
            fresh,
            format!(
                "window ends {} day(s) ago, {judged} failures judged against a gate of {gate}",
                old.map_or("?".to_string(), |d| d.to_string())
            ),
            fix_fresh,
        ),
        pass(
            "recall",
            15,
            fresh && recall.is_some_and(|v| v >= RECALL_BAR) && median.is_some_and(|m| m < 1.0),
            format!("recall {}, median selected {}", pct(recall), pct(median)),
            "raise recall to 95% with a fresh report, while the median plan selects less than everything",
        ),
    ]
}

fn firing_item(events: &[Value], today: i64) -> Item {
    let last = ["allow", "deny", "warn"]
        .iter()
        .filter_map(|d| newest(events, "write", d, today))
        .min();
    pass(
        "firing",
        5,
        last.is_some_and(|d| d <= FIRING_DAYS),
        match last {
            Some(d) => format!("last write checked {d} day(s) ago"),
            None => "no write checked by the hook".to_string(),
        },
        "install the hooks and let an agent write through them",
    )
}

fn skills_item(root: &Path, loaded: &Loaded) -> Item {
    let routed: Vec<String> = loaded
        .config
        .skills
        .routes
        .items()
        .iter()
        .map(|r| r.skill.clone())
        .collect();
    let generated = crate::knowledge::sync::generated(root);
    let tree = fairlead_lang::tree::Tree::scan(root);
    let unrouted: Vec<&String> = tree
        .files
        .iter()
        .filter(|f| f.ends_with("/SKILL.md") || f.as_str() == "SKILL.md")
        .filter(|f| !routed.contains(f) && !generated.contains(f))
        .collect();
    pass(
        "skills",
        5,
        !routed.is_empty() && unrouted.is_empty(),
        match (routed.len(), unrouted.first()) {
            (0, _) => "no routed skill".to_string(),
            (n, None) => format!("{n} routed, none left out"),
            (_, Some(f)) => format!("{} unrouted, such as {f}", unrouted.len()),
        },
        "route every SKILL.md with `[[skills.routes]]`",
    )
}

fn lessons_item(root: &Path, loaded: &Loaded) -> Item {
    let (lessons, _) = lesson::load(root, &loaded.config.memory);
    let today = lesson::today();
    let due = lessons.iter().filter(|l| l.due(&today)).count();
    pass(
        "lessons",
        5,
        !lessons.is_empty() && due == 0,
        format!("{} lesson(s), {due} due for review", lessons.len()),
        "keep at least one lesson with `fairlead learn`, and review the due ones",
    )
}

pub fn score(dir: &Path, opts: &LoadOptions, replay: Option<&Path>) -> Score {
    let loaded = config::load(dir, opts);
    let root = match &loaded {
        Ok(l) if !l.files.is_empty() => l.root.clone(),
        _ => graph_cmd::repo_root(dir),
    };
    let today = days(&lesson::today()).unwrap_or(0);
    let events: Vec<Value> = fairlead_guard::events::EventLog::open(&root)
        .map(|log| log.read())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let path = replay
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(REPLAY_REPORT));
    let report: Option<Value> = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let mut items = vec![config_item(&loaded)];
    let Ok(l) = &loaded else {
        return finish(items, events, report, today);
    };
    items.push(runners_item(&root, l));
    items.push(hooks_item(&root, l));
    items.push(done_item(&events, today));
    items.push(ci_item(&root, l));
    items.extend(replay_items(report.as_ref(), today));
    items.push(firing_item(&events, today));
    items.push(skills_item(&root, l));
    items.push(lessons_item(&root, l));
    total(items)
}

/// Scores what can be scored without a config that loads.
fn finish(mut items: Vec<Item>, events: Vec<Value>, report: Option<Value>, today: i64) -> Score {
    let invalid = || "the config doesn't load".to_string();
    let fix = "fix the config first";
    items.push(pass("runners", 15, false, invalid(), fix));
    items.push(pass("hooks", 10, false, invalid(), fix));
    items.push(done_item(&events, today));
    items.push(pass("ci", 10, false, invalid(), fix));
    items.extend(replay_items(report.as_ref(), today));
    items.push(firing_item(&events, today));
    items.push(pass("skills", 5, false, invalid(), fix));
    items.push(pass("lessons", 5, false, invalid(), fix));
    total(items)
}

fn total(items: Vec<Item>) -> Score {
    Score {
        rubric: RUBRIC,
        score: items.iter().map(|i| i.points).sum(),
        items,
    }
}

pub fn render(s: &Score) -> String {
    let mut out = format!("readiness {}/100 (rubric {})\n", s.score, s.rubric);
    for i in &s.items {
        out.push_str(&format!(
            "  [{}] {:>2}/{:<2} {:<8} {}\n",
            if i.points == i.max { "x" } else { " " },
            i.points,
            i.max,
            i.id,
            i.evidence
        ));
    }
    // Items sharing a fix, such as the two replay items, are one step.
    let mut fixes: Vec<(u32, &str)> = Vec::new();
    for i in &s.items {
        let Some(fix) = i.fix.as_deref() else {
            continue;
        };
        match fixes.iter_mut().find(|(_, f)| *f == fix) {
            Some((gain, _)) => *gain += i.max - i.points,
            None => fixes.push((i.max - i.points, fix)),
        }
    }
    fixes.sort_by_key(|(gain, _)| std::cmp::Reverse(*gain));
    if !fixes.is_empty() {
        out.push_str("next:\n");
        for (gain, fix) in fixes.iter().take(3) {
            out.push_str(&format!("  +{gain:<2} {fix}\n"));
        }
    }
    out
}

pub fn run(args: &DoctorArgs, dir: &Path, opts: &LoadOptions) -> ExitCode {
    let s = score(dir, opts, args.replay.as_deref());
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&s).expect("score prints")
        );
    } else {
        print!("{}", render(&s));
    }
    match args.min {
        Some(min) if s.score < min => ExitCode::from(1),
        _ => ExitCode::SUCCESS,
    }
}
