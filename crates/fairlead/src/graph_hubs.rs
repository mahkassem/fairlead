//! `fairlead graph hubs`: prints the files that link the most changes to
//! the most tests, why each is wide, and the barrier that would cut the top
//! one.

use std::process::ExitCode;
use std::time::Instant;

use fairlead_core::config::Config;
use fairlead_lang::Scan;
use fairlead_tests::hubs::{hubs, Hub, Hubs, Why};
use serde_json::{json, Value};

pub fn run(scan: &Scan, config: &Config, top: usize, as_json: bool, build: f64) -> ExitCode {
    let started = Instant::now();
    let found = match hubs(scan, config, top) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let seconds = (build, started.elapsed().as_secs_f64());
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&to_json(&found, seconds)).expect("hubs print")
        );
    } else {
        print!("{}", text(&found, seconds));
    }
    ExitCode::SUCCESS
}

fn barrier_line(path: &str) -> String {
    format!("graph.barrier = [{}]", Value::from(path))
}

fn why_json(why: &Why) -> Value {
    match why {
        Why::Own { kind, text } => json!({ "kind": kind, "text": text }),
        Why::Through { file, kind, text } => {
            json!({ "kind": kind, "text": text, "through": file })
        }
        Why::Plain { importers } => json!({ "kind": "plain", "importers": importers }),
    }
}

fn to_json(found: &Hubs, (build, counted): (f64, f64)) -> Value {
    let hubs: Vec<Value> = found
        .hubs
        .iter()
        .map(|h| {
            json!({
                "path": h.path, "tests": h.tests, "feeders": h.feeders,
                "score": h.score(), "why": why_json(&h.why),
            })
        })
        .collect();
    json!({
        "files": found.files,
        "tests": found.tests,
        "hubs": hubs,
        "barrier": found.hubs.first().map(|h| barrier_line(&h.path)),
        "seconds": { "build": build, "hubs": counted },
    })
}

fn why_text(why: &Why) -> String {
    match why {
        Why::Own { text, .. } => text.clone(),
        Why::Through { file, text, .. } => format!("through {file}: {text}"),
        Why::Plain { importers } => format!(
            "plain edges only; {} on it directly",
            count(*importers as u64, "file depends", "files depend")
        ),
    }
}

fn count(n: impl Into<u64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n == 1 { one } else { many })
}

fn hub_text(i: usize, hub: &Hub) -> String {
    format!(
        "{:>2}. {}\n    reaches {}, fed by {}: {}\n    wide: {}\n",
        i + 1,
        hub.path,
        count(hub.tests, "test", "tests"),
        count(hub.feeders, "file", "files"),
        count(hub.score(), "pair", "pairs"),
        why_text(&hub.why)
    )
}

fn text(found: &Hubs, (build, counted): (f64, f64)) -> String {
    let mut out = format!(
        "files that link the most changes to the most tests, of {} files and {} unit tests\n(files whose change reaches it, times the tests it reaches):\n\n",
        found.files, found.tests
    );
    let Some(first) = found.hubs.first() else {
        out.push_str("no file links a change to a test through the graph\n");
        return out;
    };
    for (i, hub) in found.hubs.iter().enumerate() {
        out.push_str(&hub_text(i, hub));
    }
    out.push_str(&format!(
        "\nto cut the top hub, so a change below it stops there and a change to it still selects its importers' tests:\n  {}\n",
        barrier_line(&first.path)
    ));
    out.push_str("`fairlead test --explain TEST` shows a selected test's chain through it, and `fairlead replay run` measures what the cut costs in recall.\n");
    out.push_str(&format!(
        "graph built in {build:.2} s, hubs counted in {counted:.2} s\n"
    ));
    out
}
