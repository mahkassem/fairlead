//! Tokens and cost per agent session, read as counts only. Claude Code's
//! transcript carries `cost-state` lines, its own running totals per process,
//! which also count subagents and its background calls; per-message usage
//! can be written before a reply finishes streaming, so it is only the
//! fallback, marked as an estimate. `claude -p --output-format json` gives
//! the same totals for a headless run. Nothing else in either file is read.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use fairlead_guard::events::{Event, EventLog, Tokens};
use serde::Serialize;
use serde_json::Value;

#[derive(Subcommand)]
pub enum TokensAction {
    /// Tokens and cost per session, from the event log.
    Report {
        /// Only this session.
        #[arg(long)]
        session: Option<String>,
        /// Print the sessions as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Record a headless run's totals: the JSON `claude -p --output-format json` prints.
    Import { file: PathBuf },
}

fn add(to: &mut Tokens, input: u64, output: u64, read: u64, write: u64) {
    to.input += input;
    to.output += output;
    to.cache_read += read;
    to.cache_write += write;
}

fn n(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Totals from `cost-state` lines: the last line of each process, summed.
fn cost_states(path: &Path) -> Option<Tokens> {
    let file = std::fs::File::open(path).ok()?;
    let mut last: BTreeMap<String, Value> = BTreeMap::new();
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        // Only these lines are parsed, so nothing else is ever held.
        if !line.contains("\"cost-state\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v["type"] == "cost-state" {
            last.insert(v["startTime"].to_string(), v);
        }
    }
    if last.is_empty() {
        return None;
    }
    let mut t = Tokens::default();
    let mut cost = 0.0;
    for state in last.values() {
        cost += state["totalCostUSD"].as_f64().unwrap_or(0.0);
        for usage in state["modelUsage"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, u)| u)
        {
            add(
                &mut t,
                n(usage, "inputTokens"),
                n(usage, "outputTokens"),
                n(usage, "cacheReadInputTokens"),
                n(usage, "cacheCreationInputTokens"),
            );
        }
    }
    t.cost_usd = Some(cost);
    Some(t)
}

/// Per-message usage, once per message: a streamed reply repeats its usage
/// on every line. Subagent transcripts beside the session's are counted too.
fn usage_lines(path: &Path) -> Option<Tokens> {
    let mut files = vec![path.to_path_buf()];
    if let (Some(dir), Some(stem)) = (path.parent(), path.file_stem()) {
        let subagents = dir.join(stem).join("subagents");
        let mut more: Vec<PathBuf> = std::fs::read_dir(subagents)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        more.sort();
        files.extend(more);
    }
    let mut seen: BTreeMap<String, (u64, u64, u64, u64)> = BTreeMap::new();
    for f in &files {
        let Ok(file) = std::fs::File::open(f) else {
            continue;
        };
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            if !line.contains("\"assistant\"") || !line.contains("\"usage\"") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let m = &v["message"];
            let Some(id) = m["id"].as_str().or_else(|| v["requestId"].as_str()) else {
                continue;
            };
            let u = &m["usage"];
            seen.insert(
                format!("{}:{id}", f.display()),
                (
                    n(u, "input_tokens"),
                    n(u, "output_tokens"),
                    n(u, "cache_read_input_tokens"),
                    n(u, "cache_creation_input_tokens"),
                ),
            );
        }
    }
    if seen.is_empty() {
        return None;
    }
    let mut t = Tokens {
        estimate: true,
        ..Tokens::default()
    };
    for (i, o, r, w) in seen.values() {
        add(&mut t, *i, *o, *r, *w);
    }
    Some(t)
}

pub fn from_transcript(path: &Path) -> Option<Tokens> {
    cost_states(path).or_else(|| usage_lines(path))
}

/// A headless run's totals, and its session id.
pub fn from_headless(json: &Value) -> Result<(Option<String>, Tokens), String> {
    if json["type"] != "result" {
        return Err("not the result JSON of `claude -p --output-format json`".into());
    }
    let mut t = Tokens::default();
    match json["modelUsage"].as_object() {
        Some(models) if !models.is_empty() => {
            for u in models.values() {
                add(
                    &mut t,
                    n(u, "inputTokens"),
                    n(u, "outputTokens"),
                    n(u, "cacheReadInputTokens"),
                    n(u, "cacheCreationInputTokens"),
                );
            }
        }
        _ => {
            let u = &json["usage"];
            add(
                &mut t,
                n(u, "input_tokens"),
                n(u, "output_tokens"),
                n(u, "cache_read_input_tokens"),
                n(u, "cache_creation_input_tokens"),
            );
        }
    }
    t.cost_usd = json["total_cost_usd"].as_f64();
    Ok((json["session_id"].as_str().map(str::to_string), t))
}

fn record(root: &Path, session: Option<String>, source: &'static str, tokens: Tokens) {
    let off = fairlead_core::config::load(
        root,
        &fairlead_core::config::LoadOptions::from_process(Vec::new()),
    )
    .is_ok_and(|l| l.config.guard.events == fairlead_core::config::Events::Off);
    if off {
        return;
    }
    if let Some(log) = EventLog::open(root) {
        let mut event = Event::new("tokens", source, std::time::Duration::ZERO);
        event.session = session;
        event.tokens = Some(tokens);
        let _ = log.append(&event);
    }
}

/// From the Stop hook's input: the session's totals so far, if its
/// transcript can be read.
pub fn record_from_hook(dir: &Path, call: &Value) {
    let Some(path) = call["transcript_path"].as_str() else {
        return;
    };
    if let Some(tokens) = from_transcript(Path::new(path)) {
        let root = crate::graph_cmd::repo_root(dir);
        record(
            &root,
            call["session_id"].as_str().map(str::to_string),
            "transcript",
            tokens,
        );
    }
}

#[derive(Serialize)]
struct Session {
    session: String,
    source: String,
    at: String,
    #[serde(flatten)]
    tokens: Tokens,
}

/// The newest totals of each session and source.
fn sessions(log: &str, only: Option<&str>) -> Vec<Session> {
    let mut latest: BTreeMap<(String, String), Session> = BTreeMap::new();
    for line in log.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["stage"] != "tokens" {
            continue;
        }
        let Ok(tokens) = serde_json::from_value::<Tokens>(v["tokens"].clone()) else {
            continue;
        };
        let session = v["session"].as_str().unwrap_or("-").to_string();
        if only.is_some_and(|o| o != session) {
            continue;
        }
        let source = v["decision"].as_str().unwrap_or("").to_string();
        let at = v["at"].as_str().unwrap_or("").to_string();
        latest.insert(
            (session.clone(), source.clone()),
            Session {
                session,
                source,
                at,
                tokens,
            },
        );
    }
    let mut out: Vec<Session> = latest.into_values().collect();
    out.sort_by(|a, b| b.at.cmp(&a.at));
    out
}

pub fn run(action: TokensAction, cwd: &Path) -> ExitCode {
    let root = crate::graph_cmd::repo_root(cwd);
    match action {
        TokensAction::Import { file } => {
            let parsed = std::fs::read_to_string(&file)
                .map_err(|e| format!("could not read {}: {e}", file.display()))
                .and_then(|t| {
                    serde_json::from_str::<Value>(&t)
                        .map_err(|_| format!("{} isn't JSON", file.display()))
                })
                .and_then(|v| from_headless(&v));
            match parsed {
                Ok((session, tokens)) => {
                    println!(
                        "recorded {} input, {} output tokens for session {}",
                        tokens.input,
                        tokens.output,
                        session.as_deref().unwrap_or("-")
                    );
                    record(&root, session, "headless", tokens);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(2)
                }
            }
        }
        TokensAction::Report { session, json } => {
            let log = EventLog::open(&root).map(|l| l.read()).unwrap_or_default();
            let found = sessions(&log, session.as_deref());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&found).expect("sessions print")
                );
                return ExitCode::SUCCESS;
            }
            if found.is_empty() {
                println!("no token counts recorded; the Stop hook records Claude Code sessions, and `fairlead tokens import` a headless run");
            }
            for s in &found {
                let cost = s
                    .tokens
                    .cost_usd
                    .map_or("cost unknown".to_string(), |c| format!("${c:.2}"));
                println!(
                    "{}  {:<10} in {}  out {}  cache read {}  cache write {}  {cost}{}",
                    s.session,
                    s.source,
                    s.tokens.input,
                    s.tokens.output,
                    s.tokens.cache_read,
                    s.tokens.cache_write,
                    if s.tokens.estimate {
                        "  (estimate)"
                    } else {
                        ""
                    }
                );
            }
            ExitCode::SUCCESS
        }
    }
}
