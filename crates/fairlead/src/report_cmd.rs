//! `fairlead report`: a draft of a report to Fairlead's makers, for the
//! project's owner to read and send. The draft is built only from numbers
//! and Fairlead's own names, and nothing here sends it anywhere.

use std::path::Path;
use std::process::ExitCode;

use fairlead_core::config::{Config, LoadOptions};
use serde_json::Value;

/// The page that says how to check and what a report may hold.
pub const PAGE: &str = "https://mahkassem.github.io/fairlead/docs/report.html";
/// Where the owner posts the draft.
pub const ISSUES: &str = "https://github.com/mahkassem/fairlead/issues/new?template=mistake.yml";
/// What a refusal ends with, so a mistake of Fairlead's reaches its makers.
pub const MISTAKE: &str = "If this looks like Fairlead's mistake and not the change's, check it first (`fairlead guide report`), then `fairlead report` drafts a report for the project's owner to review and send. Never put a secret, a private name, code or a log line in one.";

/// How many of the latest refusals a draft lists.
const KEPT: usize = 5;

#[derive(clap::Args)]
pub struct ReportArgs {
    /// Print the draft and write no file.
    #[arg(long)]
    print: bool,
}

pub fn run(args: ReportArgs, cwd: &Path) -> ExitCode {
    let text = draft(cwd);
    let file = fairlead_guard::git::common_dir(&crate::graph_cmd::repo_root(cwd))
        .map(|dir| dir.join("fairlead").join("report.md"));
    let written = match (args.print, file) {
        (false, Some(file)) => write(&file, &text).then_some(file),
        _ => None,
    };
    match written {
        Some(file) => println!(
            "report: wrote a draft to {} (inside the git folder, so it is never committed). Nothing was sent.",
            file.display()
        ),
        None => print!("{text}"),
    }
    eprintln!("report: the project's owner reads every line, fills in the three sections, and posts it at {ISSUES}");
    eprintln!("report: an agent posts it only when the owner said to in this session. An issue is public and can't be taken back, so no secret, private name, code or log line goes in. `fairlead guide report` has the checks to run first, and so has {PAGE}");
    ExitCode::SUCCESS
}

fn write(file: &Path, text: &str) -> bool {
    let made = file
        .parent()
        .is_none_or(|d| std::fs::create_dir_all(d).is_ok());
    made && std::fs::write(file, text).is_ok()
}

/// The whole draft. Every value in it is a count, a time, a state or one
/// of Fairlead's own words; a name the project chose is never copied in.
fn draft(cwd: &Path) -> String {
    let loaded = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new())).ok();
    let config = loaded.as_ref().map(|l| &l.config);
    let repo = crate::graph_cmd::repo_root(cwd);
    let mut out = String::from(HEAD);
    out.push_str(&format!(
        "- fairlead {} on {} {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    out.push_str(&format!("- how it runs: {}\n", whence(&repo)));
    out.push_str(&match config {
        Some(c) => shape(c),
        None => "- config: none loads here\n".to_string(),
    });
    let (track, items) = crate::setup_items::states(cwd);
    let states: Vec<String> = items.iter().map(|(id, s)| format!("{id} {s}")).collect();
    out.push_str(&format!("- setup ({track} track): {}\n", states.join(", ")));
    let refusals = refusals(&repo, config);
    out.push_str(&match refusals.len() {
        0 => "- no refusal is on record here\n".to_string(),
        1 => format!("- the last refusal on record:\n{}", refusals.concat()),
        n => format!(
            "- the last {n} refusals on record, newest first:\n{}",
            refusals.concat()
        ),
    });
    out.push_str(TAIL);
    out
}

const HEAD: &str = "<!-- A draft from `fairlead report`. Nothing has been sent. -->
# Fairlead said no, and it looks like its mistake

## Before this is posted

- [ ] The project's owner has read every line of this draft.
- [ ] It holds no secret: no token, password, key, cookie or connection string.
- [ ] It holds no private name: no person, customer, company, repository, host or domain.
- [ ] It holds no source code, file content, test output or log line, and no file path that says what the project is.

## What Fairlead recorded

Numbers, states and Fairlead's own words only. Runners and checks are numbered in the config's order, never named.

";

const TAIL: &str = "
## What happened

<!-- Which command or hook said no, and what it said, in your own words. Write \"a test file\" or \"the API package\" where a real name would go. -->

## What you expected

<!-- One or two sentences. -->

## How you checked it is Fairlead's mistake

<!-- The checks from `fairlead guide report` that you ran, and what each one showed. -->
";

fn whence(repo: &Path) -> &'static str {
    match (crate::hooks_cmd::package_runner(repo), crate::on_path()) {
        (Some(_), _) => "the project's own copy, through its package manager",
        (None, Some(_)) => "from the PATH",
        (None, None) => "neither a project copy nor the PATH",
    }
}

fn shape(c: &Config) -> String {
    let profile = match c.profile.is_full() {
        true => "full",
        false => "guard",
    };
    format!(
        "- config: {profile} profile; runners {}, checks {}, needs {}, `[[graph.edges]]` rules {}; budget {} s; phase {}\n",
        c.tests.runners.items().len(),
        c.checks.items().len(),
        c.needs.items().len(),
        c.graph.edges.items().len(),
        c.done.budget_seconds,
        match c.phase {
            Some(_) => "set",
            None => "not set",
        }
    )
}

/// The latest events that said no, newest first, one line each.
fn refusals(repo: &Path, config: Option<&Config>) -> Vec<String> {
    let log = fairlead_guard::events::EventLog::open(repo)
        .map(|log| log.read())
        .unwrap_or_default();
    let said_no = |e: &Value| {
        !matches!(
            e["decision"].as_str(),
            Some("allow" | "pass" | "warn") | None
        )
    };
    let events: Vec<Value> = log
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(said_no)
        .collect();
    (events.iter().rev().take(KEPT))
        .map(|e| line(e, config))
        .collect()
}

fn line(e: &Value, config: Option<&Config>) -> String {
    let word = |key: &str| e[key].as_str().unwrap_or("?").to_string();
    let mut parts = vec![format!("{} {}", word("stage"), word("decision"))];
    let rules: Vec<&str> = (e["rules"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    if !rules.is_empty() {
        parts.push(format!("rules {}", rules.join(", ")));
    }
    if let Some(n) = e["added"].as_u64().filter(|n| *n > 0) {
        parts.push(match n {
            1 => "1 finding".to_string(),
            n => format!("{n} findings"),
        });
    }
    if let Some(budget) = e["budget_seconds"].as_u64() {
        parts.push(format!("budget {budget} s"));
    }
    if let Some(reach) = e["reach"].as_object() {
        let n = |k: &str| reach.get(k).and_then(Value::as_u64).unwrap_or(0);
        parts.push(format!(
            "reached {} of {} test files, ran {}",
            n("reached"),
            n("suite"),
            n("ran")
        ));
    }
    for step in e["steps"].as_array().into_iter().flatten() {
        let outcome = match step["passed"].as_bool() {
            Some(true) => "passed",
            Some(false) => "failed",
            None => "not run",
        };
        parts.push(format!(
            "{} {outcome} in {:.1} s",
            numbered(step["id"].as_str().unwrap_or_default(), config),
            step["seconds"].as_f64().unwrap_or(0.0)
        ));
    }
    let day = e["at"].as_str().and_then(|at| at.get(..10)).unwrap_or("?");
    format!("  - {day}: {}\n", parts.join("; "))
}

/// A step by its place in the config, since its id is the project's word.
fn numbered(id: &str, config: Option<&Config>) -> String {
    let place = |ids: Vec<&str>| ids.iter().position(|i| *i == id).map(|at| at + 1);
    let Some(c) = config else {
        return "a step".to_string();
    };
    let runners = c.tests.runners.items().iter().map(|r| r.id.as_str());
    let checks = c.checks.items().iter().map(|k| k.id.as_str());
    match (place(runners.collect()), place(checks.collect()), id) {
        (Some(n), _, _) => format!("runner {n}"),
        (_, Some(n), _) => format!("check {n}"),
        (_, _, "guard") => "the guard".to_string(),
        _ => "a step".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_numbered_by_its_place_and_never_named() {
        let c: Config = toml::from_str(
            "[[tests.runners]]\nid = \"acme-billing\"\nmatch = [\"t/**\"]\ncommand = [\"x\", \"{files}\"]\n\n[[checks]]\nid = \"acme-lint\"\ncommand = [\"y\"]\n",
        )
        .unwrap();
        assert_eq!(numbered("acme-billing", Some(&c)), "runner 1");
        assert_eq!(numbered("acme-lint", Some(&c)), "check 1");
        assert_eq!(numbered("guard", Some(&c)), "the guard");
        assert_eq!(numbered("acme-secret-step", Some(&c)), "a step");
        assert_eq!(numbered("acme-billing", None), "a step");
        let event = serde_json::json!({
            "at": "2026-01-02T03:04:05Z", "stage": "done", "decision": "fail",
            "rules": [], "added": 0, "budget_seconds": 300,
            "file": "src/acme/keys.ts", "session": "s-private",
            "reach": {"all": false, "reached": 4, "suite": 9, "ran": 3},
            "steps": [{"id": "acme-billing", "passed": false, "seconds": 2.5, "files": 3}],
        });
        let text = line(&event, Some(&c));
        assert_eq!(
            text,
            "  - 2026-01-02: done fail; budget 300 s; reached 4 of 9 test files, ran 3; runner 1 failed in 2.5 s\n"
        );
        assert!(
            !text.contains("acme") && !text.contains("private"),
            "{text}"
        );
    }
}
