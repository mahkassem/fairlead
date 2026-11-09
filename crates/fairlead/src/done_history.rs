//! The coach in `fairlead done`: what CI history says about the gate's
//! steps. Each cached ledger (`ci ledger`) and each `ci run --results` file
//! kept in `.git/fairlead/results/` counts as one run; a step red in CI on
//! recent runs, and cheap, runs first, or is added when the plan left it out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fairlead_core::config::Config;
use fairlead_core::pattern::Pattern;
use fairlead_core::plan::{Invocation, InvocationKind};
use fairlead_replay::ledger::{Ledger, Verdict};

/// The newest runs read.
pub const RECENT: usize = 20;

/// One CI run: when, and the step ids that failed in it.
#[derive(Debug, Default, Clone)]
pub struct Run {
    pub at: i64,
    pub red: BTreeSet<String>,
}

/// What the history says: how many runs were read, how many each step was
/// red in, and each step's times.
#[derive(Debug, Default)]
pub struct History {
    pub runs: usize,
    pub red: BTreeMap<String, usize>,
    pub seconds: BTreeMap<String, Vec<f64>>,
}

/// Where `ci run --results` files are kept for `done` to read.
pub fn results_dir(root: &Path) -> Option<PathBuf> {
    fairlead_guard::git::common_dir(root).map(|c| c.join("fairlead").join("results"))
}

fn files(dir: Option<PathBuf>) -> Vec<PathBuf> {
    let Some(entries) = dir.and_then(|d| std::fs::read_dir(d).ok()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect()
}

fn modified(path: &Path) -> i64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// The history from the cached ledgers, the results kept, and `done`'s own
/// step times in the event log.
pub fn read(root: &Path, config: &Config, log: &str) -> History {
    let mut runs: Vec<Run> = Vec::new();
    let mut seconds: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for path in files(results_dir(root)) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(r) = serde_json::from_str::<crate::ci_report::Results>(&text) else {
            continue;
        };
        for i in &r.invocations {
            seconds.entry(i.id.clone()).or_default().push(i.seconds);
        }
        let red = r
            .invocations
            .iter()
            .filter(|i| i.failed())
            .map(|i| i.id.clone());
        runs.push(Run {
            at: modified(&path),
            red: red.collect(),
        });
    }
    for path in files(crate::ci_ledger::cache_dir(root)) {
        if let Ok(l) = crate::ci_ledger::read(&path) {
            runs.push(from_ledger(&l, config));
        }
    }
    for e in log
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
    {
        if e["stage"] != "done" {
            continue;
        }
        for s in e["steps"].as_array().into_iter().flatten() {
            if let (Some(id), Some(secs)) = (s["id"].as_str(), s["seconds"].as_f64()) {
                seconds.entry(id.to_string()).or_default().push(secs);
            }
        }
    }
    runs.sort_by_key(|r| std::cmp::Reverse(r.at));
    runs.truncate(RECENT);
    let mut red: BTreeMap<String, usize> = BTreeMap::new();
    for id in runs.iter().flat_map(|r| &r.red) {
        *red.entry(id.clone()).or_default() += 1;
    }
    History {
        runs: runs.len(),
        red,
        seconds,
    }
}

/// A ledger as one run: the steps its own failures name, by job name for
/// a check or a runner, or by the failing test's file for a runner.
pub fn from_ledger(l: &Ledger, config: &Config) -> Run {
    let at = fairlead_replay::window::epoch_seconds(&l.made_at).unwrap_or(0);
    let runners: Vec<(String, Vec<Pattern>)> = config
        .tests
        .runners
        .items()
        .iter()
        .map(|r| {
            let globs = r.matches.iter().filter_map(|g| Pattern::new(g).ok());
            (r.id.clone(), globs.collect())
        })
        .collect();
    let checks: Vec<&str> = config
        .checks
        .items()
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let mut red = BTreeSet::new();
    for f in l.failures.iter().filter(|f| f.verdict == Verdict::Own) {
        let tokens: Vec<String> = f
            .job
            .to_lowercase()
            .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect();
        let named = |id: &str| tokens.iter().any(|t| t == &id.to_lowercase());
        red.extend(checks.iter().filter(|c| named(c)).map(|c| c.to_string()));
        let file = f.file.clone().or_else(|| {
            f.test
                .as_ref()
                .map(|t| t.split(" > ").next().unwrap_or(t).trim().to_string())
        });
        for (id, globs) in &runners {
            let by_file = file
                .as_deref()
                .is_some_and(|p| globs.iter().any(|g| g.is_match(p)));
            if named(id) || by_file {
                red.insert(id.clone());
            }
        }
    }
    Run { at, red }
}

/// Whether a step is cheap: marked so, or with a median time recorded at
/// or under `done.cheap_seconds`.
fn cheap(config: &Config, h: &History, id: &str, kind: &InvocationKind) -> bool {
    let marked = match kind {
        InvocationKind::Runner => config
            .tests
            .runners
            .items()
            .iter()
            .any(|r| r.id == id && r.cheap),
        InvocationKind::Check => config.checks.items().iter().any(|c| c.id == id && c.cheap),
    };
    if marked {
        return true;
    }
    let Some(times) = h.seconds.get(id).filter(|t| !t.is_empty()) else {
        return false;
    };
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2] <= f64::from(config.done.cheap_seconds)
}

/// The steps reordered by the history, with the cheap checks it shows red
/// added, and a line for each: "typecheck: added, red in CI 6 of the last
/// 20 runs". With no history the steps are as they were.
pub fn coach(
    config: &Config,
    h: &History,
    mut steps: Vec<Invocation>,
    unplanned: impl Fn(&str) -> Result<Invocation, String>,
) -> Result<(Vec<Invocation>, Vec<String>), String> {
    if h.runs == 0 {
        return Ok((steps, Vec::new()));
    }
    let mut red: Vec<(&String, &usize)> = h.red.iter().collect();
    red.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut first: Vec<Invocation> = Vec::new();
    let mut lines = Vec::new();
    let said = |n: usize| format!("red in CI {n} of the last {} runs", h.runs);
    for (id, n) in red {
        let mine: Vec<Invocation> = steps.iter().filter(|s| &s.id == id).cloned().collect();
        if let Some(kind) = mine.first().map(|s| s.kind.clone()) {
            if cheap(config, h, id, &kind) {
                steps.retain(|s| &s.id != id);
                first.extend(mine);
                lines.push(format!("{id}: first, {}", said(*n)));
            }
            continue;
        }
        let is_check = config.checks.items().iter().any(|c| &c.id == id);
        if is_check && cheap(config, h, id, &InvocationKind::Check) {
            first.push(unplanned(id)?);
            lines.push(format!("{id}: added, {}", said(*n)));
        }
    }
    first.extend(steps);
    Ok((first, lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        toml::from_str(
            r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["t", "{files}"]

[[checks]]
id = "typecheck"
command = ["tsc"]
cheap = true

[[checks]]
id = "e2e"
command = ["slow"]
"#,
        )
        .unwrap()
    }

    fn step(id: &str, kind: InvocationKind) -> Invocation {
        Invocation {
            id: id.into(),
            kind,
            cwd: ".".into(),
            argv: vec![id.into()],
            quarantined: None,
        }
    }

    #[test]
    fn a_cheap_red_check_is_added_first_a_cheap_red_runner_moves_up_and_a_slow_one_stays() {
        let c = config();
        let h = History {
            runs: 20,
            red: BTreeMap::from([
                ("typecheck".into(), 6),
                ("unit".into(), 3),
                ("e2e".into(), 9),
            ]),
            seconds: BTreeMap::from([("unit".into(), vec![4.0, 5.0, 90.0])]),
        };
        let planned = vec![
            step("lint", InvocationKind::Check),
            step("unit", InvocationKind::Runner),
        ];
        let (steps, lines) =
            coach(&c, &h, planned, |id| Ok(step(id, InvocationKind::Check))).unwrap();
        let ids: Vec<&str> = steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["typecheck", "unit", "lint"]);
        assert_eq!(
            lines,
            [
                "typecheck: added, red in CI 6 of the last 20 runs",
                "unit: first, red in CI 3 of the last 20 runs"
            ]
        );
    }

    #[test]
    fn with_no_history_nothing_changes() {
        let planned = vec![step("lint", InvocationKind::Check)];
        let (steps, lines) = coach(&config(), &History::default(), planned, |_| {
            Err("not called".into())
        })
        .unwrap();
        assert_eq!(steps.len(), 1);
        assert!(lines.is_empty());
    }

    #[test]
    fn a_ledger_names_a_check_by_its_job_and_a_runner_by_the_failing_file() {
        let ledger: Ledger = serde_json::from_value(serde_json::json!({
            "version": 1, "repo": "o/r", "made_at": "2026-11-08T10:00:00Z",
            "head_sha": "abc", "base_branch": "main", "runs": [],
            "failures": [
                {"workflow": "ci", "run_id": 1, "attempt": 1, "job": "Typecheck (ubuntu)",
                 "message": "x", "fingerprint": "f", "verdict": "own", "why": ""},
                {"workflow": "ci", "run_id": 1, "attempt": 1, "job": "tests",
                 "test": "test/a.test.ts > adds", "message": "x", "fingerprint": "g",
                 "verdict": "own", "why": ""},
                {"workflow": "ci", "run_id": 1, "attempt": 1, "job": "e2e",
                 "message": "x", "fingerprint": "h", "verdict": "inherited", "why": ""}
            ]
        }))
        .unwrap();
        let run = from_ledger(&ledger, &config());
        let red: Vec<&str> = run.red.iter().map(String::as_str).collect();
        assert_eq!(red, ["typecheck", "unit"]);
    }
}
