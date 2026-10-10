//! `fairlead insights`: what the event log and CI's reports say about how the
//! setup works, and with `--suggest`, the config changes that evidence backs.
//! It reads through the readers that exist (the guard summary, the skills
//! report), so each count means the same here as there. Only a change that
//! flips one key is written; the rest are printed for a person to judge.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use fairlead_core::config::{self, LoadOptions, OnFinding};
use fairlead_guard::events::EventLog;
use serde::Serialize;
use serde_json::Value;

use crate::knowledge::lesson::{self, days};

/// Sessions a skill must be offered in, unused, before dropping its route is suggested.
const UNUSED_SESSIONS: usize = 10;
/// Warnings at the write stage, none reaching a commit, before denying is suggested.
const WARNINGS: usize = 20;
/// Times the same rule must be suggested for escapes or misses before it's offered.
const ESCAPES: usize = 2;

#[derive(Args)]
pub struct InsightsArgs {
    /// Only events on or after this day, YYYY-MM-DD; the last 30 days by default.
    #[arg(long, value_name = "DATE")]
    since: Option<String>,
    /// A `replay run --json-out` report, for recall, misses and recurring tests.
    #[arg(long, value_name = "FILE")]
    replay: Option<PathBuf>,
    /// A `ci run --results` file; repeat for several merges' escapes.
    #[arg(long, value_name = "FILE")]
    results: Vec<PathBuf>,
    /// Also list the config changes the evidence backs.
    #[arg(long)]
    suggest: bool,
    /// Apply the suggestions that change one key; print the rest.
    #[arg(long, requires = "suggest")]
    write: bool,
    /// Print the insights as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Serialize, Default)]
struct Insights {
    since: String,
    guard: Guard,
    done: Done,
    skills: Skills,
    lessons_never_offered: Vec<String>,
    tokens: Tokens,
    #[serde(skip_serializing_if = "Option::is_none")]
    replay: Option<Replay>,
    escapes: usize,
    suggestions: Vec<Suggestion>,
}

#[derive(Serialize, Default)]
struct Guard {
    writes: usize,
    decisions: BTreeMap<String, usize>,
    denies: BTreeMap<String, usize>,
    commit_decisions: BTreeMap<String, usize>,
}

#[derive(Serialize, Default)]
struct Done {
    passed: usize,
    failed: usize,
    /// Failing steps, by id.
    failing_steps: BTreeMap<String, usize>,
}

#[derive(Serialize, Default)]
struct Skills {
    hit_rate: Option<f64>,
    sessions_offered: usize,
    /// Offered in measured sessions and never used: name, sessions.
    unused: Vec<(String, usize)>,
    /// Used before any offer: name, sessions.
    missed: Vec<(String, usize)>,
}

#[derive(Serialize, Default)]
struct Tokens {
    sessions: usize,
    cost_usd: f64,
    estimates: usize,
}

#[derive(Serialize)]
struct Replay {
    recall: Option<f64>,
    judged: u64,
    misses: usize,
    recurring: usize,
}

#[derive(Serialize, Clone)]
struct Suggestion {
    /// What to change, as config text.
    change: String,
    /// The counts and rule behind it.
    because: String,
    /// Whether `--write` applies it.
    writes: bool,
}

fn since_default() -> String {
    let today = days(&lesson::today()).unwrap_or(0);
    date_of(today - 30)
}

/// `YYYY-MM-DD` for a day count from 1970-01-01, the inverse of `days`.
fn date_of(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|_| format!("{} isn't JSON", path.display()))
}

fn gather(root: &Path, args: &InsightsArgs, cfg: &config::Config) -> Result<Insights, String> {
    let since = args.since.clone().unwrap_or_else(since_default);
    let log = EventLog::open(root).map(|l| l.read()).unwrap_or_default();
    let recent: String = log
        .lines()
        .filter(|l| {
            serde_json::from_str::<Value>(l)
                .is_ok_and(|v| v["at"].as_str().is_some_and(|a| a >= since.as_str()))
        })
        .map(|l| format!("{l}\n"))
        .collect();
    let summary = fairlead_guard::summary::summarize(&recent);
    let skills = crate::skills_report_cmd::report(
        &log,
        Some(&since),
        None,
        crate::hooks_cmd::codex_installed(root),
    );
    let mut out = Insights {
        since: since.clone(),
        guard: Guard {
            writes: summary.writes,
            decisions: summary.decisions,
            denies: summary.denies,
            commit_decisions: summary.commit_decisions,
        },
        ..Insights::default()
    };
    let mut latest_tokens: BTreeMap<(String, String), Value> = BTreeMap::new();
    for v in recent
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
    {
        match v["stage"].as_str() {
            Some("done") => {
                if v["decision"] == "pass" {
                    out.done.passed += 1;
                } else {
                    out.done.failed += 1;
                }
                for step in v["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|s| s["passed"] == false && s["quarantined"] != true)
                {
                    *out.done
                        .failing_steps
                        .entry(step["id"].as_str().unwrap_or("?").to_string())
                        .or_default() += 1;
                }
            }
            Some("tokens") => {
                let key = (
                    v["session"].as_str().unwrap_or("-").to_string(),
                    v["decision"].as_str().unwrap_or("").to_string(),
                );
                latest_tokens.insert(key, v["tokens"].clone());
            }
            _ => {}
        }
    }
    out.tokens.sessions = latest_tokens
        .keys()
        .map(|(s, _)| s)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    for t in latest_tokens.values() {
        out.tokens.cost_usd += t["cost_usd"].as_f64().unwrap_or(0.0);
        out.tokens.estimates += usize::from(t["estimate"] == true);
    }
    out.skills = Skills {
        hit_rate: skills.skills.hit_rate,
        sessions_offered: skills.skills.sessions_offered,
        unused: skills
            .skills
            .items
            .iter()
            .filter(|r| r.unused > 0)
            .map(|r| (r.name.clone(), r.unused))
            .collect(),
        missed: skills
            .skills
            .items
            .iter()
            .filter(|r| r.missed > 0)
            .map(|r| (r.name.clone(), r.missed))
            .collect(),
    };
    let (lessons, _) = lesson::load(root, &cfg.memory);
    let offered: Vec<&str> = skills
        .lessons
        .items
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    out.lessons_never_offered = lessons
        .iter()
        .map(|l| l.front.id.clone())
        .filter(|id| !offered.contains(&id.as_str()))
        .collect();
    let mut rules: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(path) = &args.replay {
        let r = read_json(path)?;
        let misses = r["misses"].as_array().cloned().unwrap_or_default();
        for m in &misses {
            if let Some(fix) = m["fix"]
                .as_str()
                .filter(|f| f.starts_with("[[tests.owners]]"))
            {
                *rules.entry(fix.to_string()).or_default() += 1;
            }
        }
        out.replay = Some(Replay {
            recall: r["recall"].as_f64(),
            judged: r["judged"].as_u64().unwrap_or(0),
            misses: misses.len(),
            recurring: r["recurring_groups"].as_array().map_or(0, Vec::len),
        });
        if args.suggest {
            for g in r["recurring_groups"].as_array().into_iter().flatten() {
                out.suggestions.push(Suggestion {
                    change: format!(
                        "[[replay.quarantine]] path = \"{}\", job = \"^{}$\", reason = \"…\", until = \"YYYY-MM-DD\"",
                        g["path"].as_str().unwrap_or(""),
                        regex::escape(g["job"].as_str().unwrap_or(""))
                    ),
                    because: format!("it failed alike across {} unrelated pull requests; confirm it's flaky and give the reason", g["pulls"].as_array().map_or(0, Vec::len)),
                    writes: false,
                });
            }
        }
    }
    for path in &args.results {
        let r = read_json(path)?;
        for v in r["judged"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["verdict"] == "escaped")
        {
            out.escapes += 1;
            if let Some(fix) = v["fix"].as_str() {
                *rules.entry(fix.to_string()).or_default() += 1;
            }
        }
    }
    if args.suggest {
        suggest(&mut out, cfg, &rules);
    }
    Ok(out)
}

fn suggest(out: &mut Insights, cfg: &config::Config, rules: &BTreeMap<String, usize>) {
    for (rule, n) in rules.iter().filter(|(_, n)| **n >= ESCAPES) {
        out.suggestions.push(Suggestion {
            change: rule.clone(),
            because: format!("{n} escapes or misses needed this rule; add it, then `fairlead replay run` to see what it costs in plan size"),
            writes: false,
        });
    }
    for (name, n) in out
        .skills
        .unused
        .iter()
        .filter(|(_, n)| *n >= UNUSED_SESSIONS)
    {
        let used = out.skills.missed.iter().any(|(m, _)| m == name);
        if !used {
            out.suggestions.push(Suggestion {
                change: format!("drop or narrow the [[skills.routes]] entry for `{name}`"),
                because: format!("offered in {n} measured sessions and never used"),
                writes: false,
            });
        }
    }
    let warned = out.guard.decisions.get("warn").copied().unwrap_or(0);
    let reached_commit: usize = ["warn", "deny"]
        .iter()
        .map(|d| out.guard.commit_decisions.get(*d).copied().unwrap_or(0))
        .sum();
    if cfg.guard.on_finding == OnFinding::Warn && warned >= WARNINGS && reached_commit == 0 {
        out.suggestions.push(Suggestion {
            change: "[guard] on_finding = \"deny\"".into(),
            because: format!("{warned} writes were warned and no finding reached a commit, so the warnings are always fixed; denying at the write saves the round trip"),
            writes: true,
        });
    }
}

/// Sets `on_finding = "deny"` with a line edit, so the file's comments and
/// order stay as they are: the key where it is, in `[guard]` or dotted at the
/// top, else a line under `[guard]`, else a new `[guard]` table.
fn write_guard_deny(file: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| format!("could not read {}: {e}", file.display()))?;
    if file.extension().is_none_or(|e| e != "toml") {
        return Err(format!(
            "{} isn't TOML; set guard.on_finding = \"deny\" by hand",
            file.display()
        ));
    }
    let re = |p: &str| regex::Regex::new(p).expect("pattern compiles");
    let dotted = re(r#"(?m)^(\s*guard\.on_finding\s*=\s*)"warn""#);
    if dotted.is_match(&text) {
        return write(file, &dotted.replace(&text, r#"${1}"deny""#));
    }
    let Some(header) = re(r"(?m)^\[guard\][ \t]*$").find(&text) else {
        return write(
            file,
            &format!("{}\n\n[guard]\non_finding = \"deny\"\n", text.trim_end()),
        );
    };
    let end = text[header.end()..]
        .find("\n[")
        .map_or(text.len(), |i| header.end() + i);
    let section = &text[header.end()..end];
    let key = re(r#"(?m)^(\s*on_finding\s*=\s*)"warn""#);
    let section = if key.is_match(section) {
        key.replace(section, r#"${1}"deny""#).into_owned()
    } else {
        format!("\non_finding = \"deny\"{section}")
    };
    write(
        file,
        &format!("{}{section}{}", &text[..header.end()], &text[end..]),
    )
}

fn write(file: &Path, text: &str) -> Result<(), String> {
    std::fs::write(file, text).map_err(|e| format!("could not write {}: {e}", file.display()))
}

fn render(i: &Insights) -> String {
    let mut out = format!("insights since {}\n", i.since);
    let top = |m: &BTreeMap<String, usize>| {
        let mut v: Vec<(&String, &usize)> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        v.iter()
            .take(5)
            .map(|(k, n)| format!("{n} {k}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    out.push_str(&format!(
        "  guard    {} writes checked: {}\n",
        i.guard.writes,
        top(&i.guard.decisions)
    ));
    if !i.guard.denies.is_empty() {
        out.push_str(&format!(
            "           denied by rule: {}\n",
            top(&i.guard.denies)
        ));
    }
    out.push_str(&format!(
        "  done     {} passed, {} failed",
        i.done.passed, i.done.failed
    ));
    if !i.done.failing_steps.is_empty() {
        out.push_str(&format!("; failing steps: {}", top(&i.done.failing_steps)));
    }
    out.push('\n');
    let rate = i
        .skills
        .hit_rate
        .map_or("-".to_string(), |r| format!("{:.0}%", r * 100.0));
    out.push_str(&format!(
        "  skills   hit rate {rate} over {} sessions with an offer\n",
        i.skills.sessions_offered
    ));
    for (name, n) in &i.skills.unused {
        out.push_str(&format!(
            "           {name}: offered and unused in {n} session(s)\n"
        ));
    }
    for (name, n) in &i.skills.missed {
        out.push_str(&format!(
            "           {name}: used without an offer in {n} session(s)\n"
        ));
    }
    if !i.lessons_never_offered.is_empty() {
        out.push_str(&format!(
            "  lessons  never offered: {}\n",
            i.lessons_never_offered.join(", ")
        ));
    }
    out.push_str(&format!(
        "  tokens   {} session(s), ${:.2}",
        i.tokens.sessions, i.tokens.cost_usd
    ));
    if i.tokens.estimates > 0 {
        out.push_str(&format!(" ({} estimated, no cost)", i.tokens.estimates));
    }
    out.push('\n');
    if let Some(r) = &i.replay {
        let recall = r
            .recall
            .map_or("-".to_string(), |v| format!("{:.1}%", v * 100.0));
        out.push_str(&format!(
            "  replay   recall {recall} over {} failures, {} misses, {} recurring tests\n",
            r.judged, r.misses, r.recurring
        ));
    }
    if i.escapes > 0 {
        out.push_str(&format!("  escapes  {} in the results given\n", i.escapes));
    }
    if !i.suggestions.is_empty() {
        out.push_str("suggestions:\n");
        for s in &i.suggestions {
            out.push_str(&format!(
                "  {}{}\n    because {}\n",
                s.change,
                if s.writes {
                    "  (--write applies it)"
                } else {
                    ""
                },
                s.because
            ));
        }
    }
    out
}

pub fn run(args: InsightsArgs, cwd: &Path) -> ExitCode {
    let loaded = match config::load(cwd, &LoadOptions::from_process(Vec::new())) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let root = crate::graph_cmd::repo_root(cwd);
    let made = match gather(&root, &args, &loaded.config) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&made).expect("insights print")
        );
    } else {
        print!("{}", render(&made));
    }
    if args.write && made.suggestions.iter().any(|s| s.writes) {
        let Some(file) = loaded.files.first() else {
            eprintln!("no project file to write; run `fairlead init` first");
            return ExitCode::from(2);
        };
        match write_guard_deny(file) {
            Ok(()) => println!(
                "wrote guard.on_finding = \"deny\" to {}; review and commit it",
                file.display()
            ),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_count_turns_back_into_its_date() {
        for date in ["1970-01-01", "2026-03-01", "2024-02-29", "2026-10-10"] {
            assert_eq!(date_of(days(date).unwrap()), date);
        }
    }
}
