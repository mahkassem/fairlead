//! `fairlead skills report`: routing's hit rate from the event log. A brief
//! or a nudge records an `offer` naming skills and lessons, and the
//! `PostToolUse` hook records a `use` when an agent loads a skill; set
//! against each other per session, they say whether routing offers what
//! agents use.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

use fairlead_guard::events::EventLog;
use serde::Serialize;
use serde_json::Value;

/// Rows of each table shown unless `--all` is given.
const SHOWN: usize = 10;
/// A tool name only Codex sends; its hooks see edits, never a read.
const CODEX_TOOLS: [&str; 1] = ["apply_patch"];
const NONE: &str = "skills report: no offers or uses recorded yet. `fairlead brief` records what it offers, and the PostToolUse hook `fairlead hooks install` adds records the skills an agent loads.";
const LESSONS_UNMEASURED: &str = "use of lessons isn't measured: there's no event for it";

#[derive(clap::Subcommand)]
pub enum SkillsAction {
    /// Routing's hit rate from the event log: per skill and lesson, how often it was offered, used, used without an offer, and offered but not used.
    Report(ReportArgs),
}

#[derive(clap::Args)]
pub struct ReportArgs {
    /// Only events on or after this day, YYYY-MM-DD in UTC.
    #[arg(long, value_name = "DATE")]
    since: Option<String>,
    /// Only this agent session.
    #[arg(long, value_name = "ID")]
    session: Option<String>,
    /// List every skill and lesson instead of the first few.
    #[arg(long)]
    all: bool,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct SkillRow {
    pub name: String,
    /// Sessions it was offered in.
    pub offered: usize,
    /// Sessions a use named it in.
    pub used: usize,
    /// Sessions it was used in before any offer of it.
    pub missed: usize,
    /// Measured sessions it was offered in and never used.
    pub unused: usize,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct LessonRow {
    pub name: String,
    pub offered: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct Skills {
    /// Measured sessions with at least one skill offered.
    pub sessions_offered: usize,
    /// Of those, the sessions in which an offered skill was used after its offer.
    pub sessions_hit: usize,
    pub hit_rate: Option<f64>,
    pub misses: usize,
    pub items: Vec<SkillRow>,
}

#[derive(Debug, Default, Serialize)]
pub struct Lessons {
    pub sessions_offered: usize,
    pub use_measured: bool,
    pub items: Vec<LessonRow>,
}

#[derive(Debug, Default, Serialize)]
pub struct Unmeasured {
    /// Agents whose installed hooks can't see a skill being loaded.
    pub agents: Vec<String>,
    /// Sessions of such an agent: their offers count, their non-use doesn't.
    pub sessions: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub since: Option<String>,
    pub session: Option<String>,
    /// Sessions with an offer or a use in the window.
    pub sessions: usize,
    /// Offers made with no session, which no use can be set against.
    pub offers_without_session: usize,
    pub skills: Skills,
    pub lessons: Lessons,
    pub unmeasured: Unmeasured,
}

/// The first offer and the first use of one item in one session, by line.
#[derive(Default, Clone, Copy)]
struct Seen {
    offer: Option<usize>,
    used: Option<usize>,
}

pub fn run(action: SkillsAction, cwd: &Path) -> ExitCode {
    match action {
        SkillsAction::Report(args) => run_report(&args, cwd),
    }
}

fn run_report(args: &ReportArgs, cwd: &Path) -> ExitCode {
    if let Some(day) = &args.since {
        if !is_day(day) {
            eprintln!("skills report: --since takes a day as YYYY-MM-DD, not {day}");
            return ExitCode::from(2);
        }
    }
    let root = crate::graph_cmd::repo_root(cwd);
    let Some(log) = EventLog::open(&root) else {
        eprintln!("skills report: not a git repository; the event log lives in .git/fairlead");
        return ExitCode::from(2);
    };
    let text = log.read();
    let codex = crate::hooks_cmd::codex_installed(&root);
    let made = report(&text, args.since.as_deref(), args.session.as_deref(), codex);
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&made).expect("the report serializes")
        );
    } else if !text.lines().any(is_routing) {
        println!("{NONE}");
    } else {
        print!("{}", render(&made, args.all));
    }
    ExitCode::SUCCESS
}

fn is_day(day: &str) -> bool {
    let b = day.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn is_routing(line: &str) -> bool {
    serde_json::from_str::<Value>(line).is_ok_and(|e| e["stage"] == "offer" || e["stage"] == "use")
}

/// Sessions that sent a tool only an agent with no read-visible hook sends,
/// from the whole log, so a window that starts mid-session still knows.
fn codex_sessions(events: &[Value]) -> BTreeSet<String> {
    events
        .iter()
        .filter(|e| e["tool"].as_str().is_some_and(|t| CODEX_TOOLS.contains(&t)))
        .filter_map(|e| e["session"].as_str().map(String::from))
        .collect()
}

/// Each session's items with their first offer and first use, oldest first.
fn sessions(
    events: &[Value],
    since: Option<&str>,
    only: Option<&str>,
) -> (BTreeMap<String, BTreeMap<String, Seen>>, usize) {
    let mut out: BTreeMap<String, BTreeMap<String, Seen>> = BTreeMap::new();
    let mut unsessioned = 0;
    for (line, e) in events.iter().enumerate() {
        let stage = e["stage"].as_str().unwrap_or("");
        if stage != "offer" && stage != "use" {
            continue;
        }
        let day = e["at"].as_str().and_then(|a| a.get(..10)).unwrap_or("");
        if since.is_some_and(|s| day < s) {
            continue;
        }
        let Some(session) = e["session"].as_str() else {
            unsessioned += usize::from(stage == "offer" && only.is_none());
            continue;
        };
        if only.is_some_and(|o| o != session) {
            continue;
        }
        let items = out.entry(session.to_string()).or_default();
        for item in e["items"].as_array().into_iter().flatten() {
            let Some(item) = item.as_str() else { continue };
            let seen = items.entry(item.to_string()).or_default();
            let slot = if stage == "offer" {
                &mut seen.offer
            } else {
                &mut seen.used
            };
            slot.get_or_insert(line);
        }
    }
    (out, unsessioned)
}

/// The report over the log's lines from `since` (a UTC day), for one
/// session or all; `codex` says whether Codex's hooks are installed.
pub fn report(log: &str, since: Option<&str>, only: Option<&str>, codex: bool) -> Report {
    let events: Vec<Value> = log
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let blind = codex_sessions(&events);
    let (by_session, unsessioned) = sessions(&events, since, only);
    let mut skills: BTreeMap<String, SkillRow> = BTreeMap::new();
    let mut lessons: BTreeMap<String, LessonRow> = BTreeMap::new();
    let mut made = Report {
        since: since.map(String::from),
        session: only.map(String::from),
        sessions: by_session.len(),
        offers_without_session: unsessioned,
        ..Report::default()
    };
    let mut lesson_sessions = 0;
    for (session, items) in &by_session {
        let measured = !blind.contains(session);
        if !measured {
            made.unmeasured.sessions.push(session.clone());
        }
        let (mut offered_skill, mut hit) = (false, false);
        for (item, seen) in items {
            if let Some(name) = item.strip_prefix("lesson:") {
                let row = lessons.entry(name.to_string()).or_default();
                row.offered += usize::from(seen.offer.is_some());
                continue;
            }
            let Some(name) = item.strip_prefix("skill:") else {
                continue;
            };
            let row = skills.entry(name.to_string()).or_default();
            row.offered += usize::from(seen.offer.is_some());
            row.used += usize::from(seen.used.is_some());
            match (seen.offer, seen.used) {
                (offer, Some(used)) if offer.is_none_or(|o| used < o) => row.missed += 1,
                (Some(_), Some(_)) => hit = true,
                (Some(_), None) if measured => row.unused += 1,
                _ => {}
            }
            offered_skill |= seen.offer.is_some();
        }
        if offered_skill && measured {
            made.skills.sessions_offered += 1;
            made.skills.sessions_hit += usize::from(hit);
        }
        lesson_sessions += usize::from(
            items
                .iter()
                .any(|(i, s)| i.starts_with("lesson:") && s.offer.is_some()),
        );
    }
    made.skills.misses = skills.values().map(|r| r.missed).sum();
    made.skills.hit_rate = (made.skills.sessions_offered > 0)
        .then(|| made.skills.sessions_hit as f64 / made.skills.sessions_offered as f64);
    made.skills.items = sorted(skills, |r| r.offered, |r, n| r.name = n);
    made.lessons.sessions_offered = lesson_sessions;
    made.lessons.items = sorted(lessons, |r| r.offered, |r, n| r.name = n);
    if codex {
        made.unmeasured.agents.push("codex".into());
    }
    made
}

/// Rows by offers, most first, then by name.
fn sorted<R>(
    rows: BTreeMap<String, R>,
    offered: impl Fn(&R) -> usize,
    name: impl Fn(&mut R, String),
) -> Vec<R> {
    let mut out: Vec<(String, R)> = rows.into_iter().collect();
    out.sort_by(|a, b| offered(&b.1).cmp(&offered(&a.1)).then(a.0.cmp(&b.0)));
    out.into_iter()
        .map(|(n, mut r)| {
            name(&mut r, n);
            r
        })
        .collect()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The report as text: totals, the skills table and the lessons table,
/// each capped at `SHOWN` rows unless `all`.
pub fn render(r: &Report, all: bool) -> String {
    let window = match (&r.since, &r.session) {
        (Some(d), Some(s)) => format!("since {d}, session {s}"),
        (Some(d), None) => format!("since {d}"),
        (None, Some(s)) => format!("session {s}"),
        (None, None) => "all recorded events".to_string(),
    };
    let mut out = format!(
        "skills report ({window}): {}\n",
        plural(r.sessions, "session", "sessions")
    );
    if r.sessions == 0 {
        out.push_str("  no offers or uses in this window\n");
        return out;
    }
    let s = &r.skills;
    let rate = s
        .hit_rate
        .map(|h| format!("{:.0}%", h * 100.0))
        .unwrap_or_else(|| "n/a".into());
    out.push_str(&format!(
        "skills: offered in {}, an offered skill used in {} (hit rate {rate}); {} (used before any offer)\n",
        plural(s.sessions_offered, "session", "sessions"),
        s.sessions_hit,
        plural(s.misses, "miss", "misses"),
    ));
    if !r.unmeasured.agents.is_empty() || !r.unmeasured.sessions.is_empty() {
        // The sessions are told by Codex's own tool, so they name it uninstalled too.
        let who = if r.unmeasured.agents.is_empty() {
            "codex".to_string()
        } else {
            r.unmeasured.agents.join(", ")
        };
        out.push_str(&format!(
            "unmeasured: {who}, whose hooks don't see a skill load; {} not counted as unused\n",
            plural(r.unmeasured.sessions.len(), "session", "sessions")
        ));
    }
    if r.offers_without_session > 0 {
        out.push_str(&format!(
            "{} with no session, not counted\n",
            plural(r.offers_without_session, "offer", "offers")
        ));
    }
    let cap = |n: usize| if all { n } else { n.min(SHOWN) };
    if !s.items.is_empty() {
        out.push_str(&format!(
            "\n  {:<24} {:>7} {:>5} {:>6} {:>6}\n",
            "skill", "offered", "used", "missed", "unused"
        ));
        for row in &s.items[..cap(s.items.len())] {
            out.push_str(&format!(
                "  {:<24} {:>7} {:>5} {:>6} {:>6}\n",
                row.name, row.offered, row.used, row.missed, row.unused
            ));
        }
        more(&mut out, s.items.len(), cap(s.items.len()));
    }
    let l = &r.lessons;
    out.push_str(&format!(
        "\nlessons: offered in {}; {LESSONS_UNMEASURED}\n",
        plural(l.sessions_offered, "session", "sessions")
    ));
    if !l.items.is_empty() {
        out.push_str(&format!("  {:<24} {:>7}\n", "lesson", "offered"));
        for row in &l.items[..cap(l.items.len())] {
            out.push_str(&format!("  {:<24} {:>7}\n", row.name, row.offered));
        }
        more(&mut out, l.items.len(), cap(l.items.len()));
    }
    out
}

fn more(out: &mut String, total: usize, shown: usize) {
    if shown < total {
        out.push_str(&format!("  … {} more (--all)\n", total - shown));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_four_two_and_two_digits() {
        assert!(is_day("2026-10-06"));
        assert!(!is_day("2026-1-06") && !is_day("06-10-2026") && !is_day("2026/10/06"));
    }
}
