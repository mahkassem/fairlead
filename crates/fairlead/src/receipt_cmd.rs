//! `fairlead receipt` and `fairlead next`. The receipt compares what changed
//! with the session's brief and the done gate: files the brief named, files
//! in its reach, and files outside it with the tests they add. `next` is the
//! one step the change loop is waiting for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use fairlead_core::plan::Reason;
use fairlead_guard::events::{timestamp, EventLog};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::brief_cmd::{Brief, Store};
use crate::plan_cmd::{make, Changes, Planned};

const SHOWN: usize = 5;

#[derive(clap::Args)]
pub struct ReceiptArgs {
    /// The branch or commit to compare with when the session has no brief.
    #[arg(long)]
    base: Option<String>,
    /// The agent session whose brief to compare with, over `CLAUDE_CODE_SESSION_ID`.
    #[arg(long)]
    session: Option<String>,
    /// Also write the receipt to this file: JSON for a `.json` name, else text.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Print the receipt as JSON.
    #[arg(long)]
    json: bool,
    /// List every file and test instead of the first few.
    #[arg(long)]
    all: bool,
}

#[derive(clap::Args)]
pub struct NextArgs {
    /// The branch or commit to compare with when the session has no brief.
    #[arg(long)]
    base: Option<String>,
    /// The agent session, over `CLAUDE_CODE_SESSION_ID`.
    #[arg(long)]
    session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub tree: String,
    pub written_at: String,
    /// Changed files the brief named.
    pub named: Vec<String>,
    /// Changed files the brief's paths reach through the graph.
    pub reached: Vec<String>,
    /// Changed files outside the brief's reach, with the tests each adds.
    pub outside: Vec<Outside>,
    pub tests_now: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests_briefed: Option<usize>,
    pub gate: Gate,
    pub next: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outside {
    pub path: String,
    pub adds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gate {
    /// `passed`, `failed` or `not run` for the tree as it is now.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_step: Option<String>,
    pub steps: usize,
    /// Steps that failed as their `[[quarantine]]` entry expects, so a pass
    /// with one doesn't read as a clean pass.
    #[serde(default)]
    pub held: usize,
    pub seconds: f64,
}

pub fn run(args: ReceiptArgs, cwd: &Path) -> ExitCode {
    match receipt(&args, cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("receipt: {e}");
            ExitCode::from(2)
        }
    }
}

pub fn run_next(args: NextArgs, cwd: &Path) -> ExitCode {
    let session = session(args.session.clone());
    match state(cwd, args.base.clone(), session.as_deref()) {
        Ok(s) => {
            println!("{}", next_line(&s));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("next: {e}");
            ExitCode::from(2)
        }
    }
}

fn session(given: Option<String>) -> Option<String> {
    given.or_else(|| std::env::var(crate::brief_cmd::SESSION_ENV).ok())
}

/// Where the change loop stands: the brief, the plan for what changed, the
/// gate's record for this tree and whether a receipt was written for it.
pub(crate) struct State {
    pub(crate) brief: Option<Brief>,
    pub(crate) now: Planned,
    pub(crate) changed: Vec<String>,
    pub(crate) gate: Gate,
    receipt_written: bool,
    /// The failing step's command, when the gate failed.
    failed_command: Option<String>,
}

fn state(cwd: &Path, base: Option<String>, session: Option<&str>) -> Result<State, String> {
    let root = crate::graph_cmd::repo_root(cwd);
    let brief = Store::open(&root).and_then(|s| s.current(session));
    state_for(cwd, brief, base)
}

/// Where the change loop stands for a brief found some other way, such as
/// the newest on the branch.
pub(crate) fn state_for(
    cwd: &Path,
    brief: Option<Brief>,
    base: Option<String>,
) -> Result<State, String> {
    let root = crate::graph_cmd::repo_root(cwd);
    let store = Store::open(&root);
    let base = brief.as_ref().and_then(|b| b.base.clone()).or(base);
    let now = make(cwd, &Changes::new(base, Vec::new(), Vec::new()))?;
    let changed: Vec<String> = now.plan.changed.iter().map(|c| c.path.clone()).collect();
    let tree = now.plan.tree_hash.clone();
    let log = EventLog::open(&root).map(|l| l.read()).unwrap_or_default();
    let gate = gate(&log, &tree);
    let failed_command = gate.failed_step.as_ref().and_then(|id| {
        crate::done_cmd::steps(&now)
            .ok()?
            .into_iter()
            .find(|s| &s.id == id)
            .map(|s| s.argv.join(" "))
    });
    let receipt_written = store.as_ref().is_some_and(|s| {
        s.receipt(brief.as_ref().map(|b| b.id.as_str()))
            .is_some_and(|r| r.tree == tree && r.gate.state == "passed")
    });
    Ok(State {
        brief,
        now,
        changed,
        gate,
        receipt_written,
        failed_command,
    })
}

/// The newest `done` run for `tree`, as the receipt shows it.
fn gate(log: &str, tree: &str) -> Gate {
    let event = log
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|e| e["stage"] == "done" && e["tree"] == tree);
    let Some(e) = event else {
        return Gate {
            state: "not run".into(),
            at: None,
            failed_step: None,
            steps: 0,
            held: 0,
            seconds: 0.0,
        };
    };
    let steps = e["steps"].as_array().cloned().unwrap_or_default();
    let passed = e["decision"] == "pass";
    Gate {
        state: if passed { "passed" } else { "failed" }.into(),
        at: e["at"].as_str().map(String::from),
        failed_step: steps
            .iter()
            .find(|s| s["passed"] == false && s["quarantined"] != true)
            .and_then(|s| s["id"].as_str().map(String::from)),
        steps: steps.len(),
        held: steps.iter().filter(|s| s["quarantined"] == true).count(),
        seconds: steps.iter().filter_map(|s| s["seconds"].as_f64()).sum(),
    }
}

/// The one step that's due, in the words the Stop hook and the note use too.
pub(crate) fn next_line(s: &State) -> String {
    if s.changed.is_empty() {
        return "next: nothing due; nothing has changed".into();
    }
    if s.brief.is_none() {
        return "next: brief: `fairlead brief <paths>` lists what the change reaches".into();
    }
    match s.gate.state.as_str() {
        "passed" if s.receipt_written => {
            "next: nothing due; the gate passed and the receipt is written".into()
        }
        "passed" => "next: receipt: `fairlead receipt` compares the change with its brief".into(),
        "failed" => match &s.failed_command {
            Some(cmd) => format!("next: fix the failing step, `{cmd}`, then `fairlead done`"),
            None => "next: fix what `fairlead done` reported, then run it again".into(),
        },
        _ => "next: done: `fairlead done` hasn't passed for the tree as it is now".into(),
    }
}

fn receipt(args: &ReceiptArgs, cwd: &Path) -> Result<(), String> {
    let root = crate::graph_cmd::repo_root(cwd);
    let s = state(
        cwd,
        args.base.clone(),
        session(args.session.clone()).as_deref(),
    )?;
    let mut made = compare(&s, &root, cwd)?;
    made.next = next_line(&State {
        receipt_written: true,
        ..s
    });
    if let Some(store) = Store::open(&root) {
        store.save_receipt(&made)?;
    }
    let text = text(&made, args.all);
    if let Some(out) = &args.out {
        let body = if out.extension().is_some_and(|e| e == "json") {
            serde_json::to_string_pretty(&made).expect("receipt serializes") + "\n"
        } else {
            text.clone()
        };
        std::fs::write(out, body).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&made).expect("receipt serializes")
        );
    } else {
        print!("{text}");
    }
    Ok(())
}

fn compare(s: &State, root: &Path, cwd: &Path) -> Result<Receipt, String> {
    let mut named = Vec::new();
    let mut reached = Vec::new();
    let mut outside_paths = Vec::new();
    let mut briefed_tests: Option<BTreeSet<String>> = None;
    if let Some(brief) = &s.brief {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let files = brief
            .paths
            .iter()
            .map(|p| root.join(p).display().to_string())
            .collect();
        let briefed = make(cwd, &Changes::new(brief.base.clone(), files, Vec::new()))?;
        let graph = &briefed.scan.graph;
        let ids: Vec<u32> = brief.paths.iter().filter_map(|p| graph.id(p)).collect();
        let reach: BTreeSet<&str> = graph
            .affected(&ids)
            .keys()
            .map(|&i| graph.files[i as usize].as_str())
            .collect();
        let names: BTreeSet<&str> = brief.paths.iter().map(String::as_str).collect();
        for p in &s.changed {
            if names.contains(p.as_str()) {
                named.push(p.clone());
            } else if reach.contains(p.as_str()) {
                reached.push(p.clone());
            } else {
                outside_paths.push(p.clone());
            }
        }
        briefed_tests = Some(briefed.plan.tests.iter().map(|t| t.path.clone()).collect());
    } else {
        outside_paths = s.changed.clone();
    }
    let mut adds: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(briefed) = &briefed_tests {
        for t in &s.now.plan.tests {
            if briefed.contains(&t.path) {
                continue;
            }
            if let Some(cause) = cause(&t.reason, &t.path) {
                adds.entry(cause).or_default().push(t.path.clone());
            }
        }
    }
    let outside = outside_paths
        .into_iter()
        .map(|path| Outside {
            adds: adds.remove(&path).unwrap_or_default(),
            path,
        })
        .collect();
    Ok(Receipt {
        version: 1,
        brief: s.brief.as_ref().map(|b| b.id.clone()),
        base: s.now.plan.base.clone(),
        tree: s.now.plan.tree_hash.clone(),
        written_at: timestamp(SystemTime::now()),
        named,
        reached,
        outside,
        tests_now: s.now.plan.tests.len(),
        tests_briefed: briefed_tests.map(|t| t.len()),
        gate: s.gate.clone(),
        next: String::new(),
    })
}

/// The changed file that put a test in the plan.
fn cause(reason: &Reason, test: &str) -> Option<String> {
    match reason {
        Reason::Import { chain } => chain.first().cloned(),
        Reason::Owner { changed, .. } => Some(changed.clone()),
        Reason::Changed => Some(test.to_string()),
        Reason::RunAll { path } | Reason::Unreached { path, .. } => Some(path.clone()),
        _ => None,
    }
}

pub fn text(r: &Receipt, all: bool) -> String {
    let mut out = String::new();
    let base = r
        .base
        .as_deref()
        .map_or("none".into(), |c| c.chars().take(7).collect::<String>());
    match &r.brief {
        Some(id) => out.push_str(&format!("receipt for brief {id}  base {base}..worktree\n")),
        None => out.push_str(&format!(
            "receipt  base {base}..worktree  no brief for this session, so nothing to compare against\n"
        )),
    }
    let total = r.named.len() + r.reached.len() + r.outside.len();
    if r.brief.is_some() {
        out.push_str(&format!(
            "changed  {total} file{}: {} named in the brief, {} in its reach, {} outside\n",
            if total == 1 { "" } else { "s" },
            r.named.len(),
            r.reached.len(),
            r.outside.len()
        ));
    } else {
        out.push_str(&format!(
            "changed  {total} file{}\n",
            if total == 1 { "" } else { "s" }
        ));
    }
    let shown = if all {
        r.outside.len()
    } else {
        r.outside.len().min(SHOWN)
    };
    let label = if r.brief.is_some() {
        "outside"
    } else {
        "changed"
    };
    for o in &r.outside[..shown] {
        let adds = match o.adds.len() {
            0 => String::new(),
            n => {
                let first: Vec<&str> = o.adds.iter().take(2).map(String::as_str).collect();
                let more = if n > first.len() { ", …" } else { "" };
                format!(
                    "  adds {n} test{} ({}{more})",
                    if n == 1 { "" } else { "s" },
                    first.join(", ")
                )
            }
        };
        out.push_str(&format!("  {label:<8} {}{adds}\n", o.path));
    }
    if shown < r.outside.len() {
        out.push_str(&format!(
            "  … {} more (fairlead receipt --all)\n",
            r.outside.len() - shown
        ));
    }
    match r.tests_briefed {
        Some(briefed) => {
            let added = r.outside.iter().map(|o| o.adds.len()).sum::<usize>();
            out.push_str(&format!(
                "tests    planned now {}, briefed {briefed}: {added} added by files outside the brief\n",
                r.tests_now
            ));
        }
        None => out.push_str(&format!("tests    planned now {}\n", r.tests_now)),
    }
    let gate = gate_line(&r.gate);
    out.push_str(&format!("gate     {gate}\n"));
    out.push_str(&r.next.replacen("next: ", "next     ", 1));
    out.push('\n');
    out
}

/// The gate's state in words; a pass says how many steps were held.
pub(crate) fn gate_line(g: &Gate) -> String {
    match g.state.as_str() {
        "passed" => format!(
            "passed for this tree at {} ({} step{}{}) in {:.0} s",
            g.at.as_deref().map_or("?", |a| a.get(11..16).unwrap_or(a)),
            g.steps,
            if g.steps == 1 { "" } else { "s" },
            if g.held > 0 {
                format!(", {} held", g.held)
            } else {
                String::new()
            },
            g.seconds
        ),
        "failed" => format!(
            "failed for this tree at {}",
            g.failed_step.as_deref().unwrap_or("a step")
        ),
        _ => "not run for this tree".into(),
    }
}

impl Store {
    fn receipts(&self) -> PathBuf {
        self.dir()
            .parent()
            .expect("briefs live in a directory")
            .join("receipts")
    }

    fn save_receipt(&self, r: &Receipt) -> Result<(), String> {
        let dir = self.receipts();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(format!("{}.json", r.brief.as_deref().unwrap_or("none")));
        let text = serde_json::to_string_pretty(r).expect("receipt serializes") + "\n";
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The receipt last written for a brief, or for changes with none.
    fn receipt(&self, brief: Option<&str>) -> Option<Receipt> {
        let path = self
            .receipts()
            .join(format!("{}.json", brief.unwrap_or("none")));
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_reads_the_newest_done_run_for_this_tree_and_its_failing_step() {
        let log = [
            r#"{"stage":"done","tree":"t1","decision":"pass","at":"2026-09-29T14:02:00.000Z","steps":[{"id":"unit","passed":true,"seconds":9.5}]}"#,
            r#"{"stage":"done","tree":"t1","decision":"fail","at":"2026-09-29T14:05:00.000Z","steps":[{"id":"unit","passed":true,"seconds":1.0},{"id":"lint","passed":false,"seconds":0.5}]}"#,
        ]
        .join("\n");
        let g = gate(&log, "t1");
        assert_eq!(g.state, "failed");
        assert_eq!(g.failed_step.as_deref(), Some("lint"));
        assert_eq!(g.steps, 2);
        assert_eq!(g.held, 0);
        assert_eq!(gate(&log, "t2").state, "not run");
    }

    #[test]
    fn a_pass_that_excused_a_quarantined_failure_counts_it_as_held() {
        let log = r#"{"stage":"done","tree":"t1","decision":"pass","at":"2026-09-29T14:02:00.000Z","steps":[{"id":"unit","passed":true,"seconds":9.5},{"id":"unit","passed":false,"quarantined":true,"seconds":1.5},{"id":"lint","passed":true,"seconds":1.0}]}"#;
        let g = gate(log, "t1");
        assert_eq!((g.state.as_str(), g.steps, g.held), ("passed", 3, 1));
        assert_eq!(g.failed_step, None);
        assert!(
            gate_line(&g).contains("(3 steps, 1 held)"),
            "{}",
            gate_line(&g)
        );
        let clean = Gate { held: 0, ..g };
        assert!(
            gate_line(&clean).contains("(3 steps)"),
            "{}",
            gate_line(&clean)
        );
    }

    #[test]
    fn a_receipt_written_before_held_was_counted_still_reads() {
        let old = r#"{"state":"passed","at":"2026-09-29T14:02:00.000Z","steps":2,"seconds":10.5}"#;
        let g: Gate = serde_json::from_str(old).expect("an old gate parses");
        assert_eq!((g.steps, g.held), (2, 0));
    }

    #[test]
    fn a_test_is_put_down_to_the_changed_file_that_selected_it() {
        let chain = vec!["src/b.ts".to_string(), "test/b.test.ts".to_string()];
        assert_eq!(
            cause(&Reason::Import { chain }, "test/b.test.ts").as_deref(),
            Some("src/b.ts")
        );
        assert_eq!(
            cause(&Reason::Changed, "test/c.test.ts").as_deref(),
            Some("test/c.test.ts")
        );
        assert_eq!(cause(&Reason::Canary, "t"), None);
    }
}
