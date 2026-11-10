//! The `proved` step of `fairlead setup`: each runner runs its smallest test
//! file once, under a time limit, and the result is kept in the git
//! directory, so `setup --check` and `doctor` read it without running
//! anything.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use fairlead_core::config::{Config, Runner};
use fairlead_core::pattern::Pattern;
use fairlead_core::plan::{Invocation, Reason, TestSelection};
use fairlead_lang::tree::Tree;
use fairlead_tests::modules::Modules;
use fairlead_tests::testfiles;
use serde::{Deserialize, Serialize};

/// How long one runner's one test file may take.
pub const LIMIT: Duration = Duration::from_secs(60);
const FILE: &str = "setup.json";

/// What running one test file of a runner showed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proof {
    pub runner: String,
    pub file: String,
    /// None when the runner couldn't start here: a need isn't met, or its
    /// tool isn't installed.
    pub passed: Option<bool>,
    pub seconds: f64,
    /// Why it didn't pass.
    pub why: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub proved: Vec<Proof>,
}

fn path(repo: &Path) -> Option<PathBuf> {
    fairlead_guard::git::git_dir(repo).map(|dir| dir.join("fairlead").join(FILE))
}

/// The last results, empty when setup never proved anything here.
pub fn read(repo: &Path) -> Record {
    path(repo)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write(repo: &Path, record: &Record) -> Result<(), String> {
    let path = path(repo).ok_or("not inside a git repository")?;
    let dir = path.parent().expect("the record has a directory");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(record).expect("the record serializes") + "\n";
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// One runner's test to run: its smallest file, by bytes, and the
/// invocation that names it.
pub struct Chosen {
    pub runner: String,
    pub file: String,
    pub inv: Invocation,
}

/// What setup does about one runner that has a test file.
enum Step {
    /// It passed before, and that result stands.
    Kept(Proof),
    Run(Chosen),
    /// Nothing of it can run here, and why.
    Blocked(Proof),
}

/// A step for each runner with a test file, in the config's order: the
/// smallest file `usable` lets through, unless `old` holds a pass.
fn steps(
    tree: &Tree,
    config: &Config,
    old: &Record,
    usable: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Vec<Step>, String> {
    let packages = fairlead_lang::workspace::discover(tree);
    let modules = Modules::discover(tree, &packages, &config.modules)?;
    let found = testfiles::discover(tree, config, &modules)?;
    let size = |path: &str| std::fs::metadata(tree.abs(path)).map_or(u64::MAX, |m| m.len());
    let mut out = Vec::new();
    for (index, runner) in config.tests.runners.items().iter().enumerate() {
        let mut mine: Vec<&testfiles::TestFile> = found
            .tests
            .iter()
            .filter(|t| t.runner == Some(index))
            .collect();
        mine.sort_by_key(|t| (size(&t.path), t.path.clone()));
        let Some(smallest) = mine.first() else {
            continue;
        };
        let passed = |p: &&Proof| p.runner == runner.id && p.passed == Some(true);
        if let Some(kept) = old.proved.iter().find(passed) {
            out.push(Step::Kept(kept.clone()));
            continue;
        }
        let pick = mine.iter().find(|t| usable(&t.path).is_ok());
        out.push(match pick.and_then(|t| invocation(&modules, runner, t)) {
            Some(chosen) => Step::Run(chosen),
            None => Step::Blocked(Proof {
                runner: runner.id.clone(),
                file: smallest.path.clone(),
                passed: None,
                seconds: 0.0,
                why: Some(
                    usable(&smallest.path)
                        .err()
                        .unwrap_or_else(|| "its command names no test".into()),
                ),
            }),
        });
    }
    Ok(out)
}

fn invocation(modules: &Modules, runner: &Runner, test: &testfiles::TestFile) -> Option<Chosen> {
    let selection = TestSelection {
        path: test.path.clone(),
        module: test.module.map(|m| modules.get(m).name.clone()),
        runner: Some(runner.id.clone()),
        class: test.class,
        reason: Reason::Changed,
    };
    let inv = fairlead_tests::invoke::naming(modules, runner, &[&selection])
        .into_iter()
        .next()?;
    Some(Chosen {
        runner: runner.id.clone(),
        file: test.path.clone(),
        inv,
    })
}

/// What `prove` would run, with no need checked: for `setup --dry-run`.
pub fn plan(repo: &Path, tree: &Tree, config: &Config) -> Result<Vec<Chosen>, String> {
    Ok(steps(tree, config, &read(repo), &|_| Ok(()))?
        .into_iter()
        .filter_map(|step| match step {
            Step::Run(chosen) => Some(chosen),
            _ => None,
        })
        .collect())
}

/// Each need that isn't met on this machine, with the test files it covers.
fn unmet_needs(tree: &Tree, config: &Config) -> Vec<(String, Vec<Pattern>)> {
    let needs: Vec<_> = config
        .needs
        .items()
        .iter()
        .filter(|n| !n.tests.is_empty())
        .cloned()
        .collect();
    let checked = crate::needs::check_all(&needs, &tree.root);
    needs
        .iter()
        .zip(checked)
        .filter(|(_, c)| !c.met)
        .map(|(n, _)| {
            let globs = n.tests.iter().filter_map(|g| Pattern::new(g).ok());
            (n.id.clone(), globs.collect())
        })
        .collect()
}

/// Runs one test file of each runner that has no passing result on record,
/// keeps the results, and returns how many runners it ran or tried to.
pub fn prove(repo: &Path, tree: &Tree, config: &Config, limit: Duration) -> Result<usize, String> {
    let old = read(repo);
    // Checked once, and only when a runner has something left to prove.
    let unmet: std::cell::OnceCell<Vec<(String, Vec<Pattern>)>> = std::cell::OnceCell::new();
    let usable = |path: &str| {
        let unmet = unmet.get_or_init(|| unmet_needs(tree, config));
        match unmet
            .iter()
            .find(|(_, globs)| globs.iter().any(|p| p.is_match(path)))
        {
            Some((need, _)) => Err(format!("need \"{need}\" not met")),
            None => Ok(()),
        }
    };
    let mut record = Record::default();
    let mut ran = 0;
    for step in steps(tree, config, &old, &usable)? {
        record.proved.push(match step {
            Step::Kept(proof) => proof,
            Step::Blocked(proof) => {
                ran += 1;
                proof
            }
            Step::Run(chosen) => {
                ran += 1;
                eprintln!(
                    "setup: running {} on {} ({} s limit)",
                    chosen.runner,
                    chosen.file,
                    limit.as_secs()
                );
                let (passed, seconds, why) = run_one(&tree.root, &chosen.inv, limit);
                Proof {
                    runner: chosen.runner,
                    file: chosen.file,
                    passed,
                    seconds,
                    why,
                }
            }
        });
    }
    if ran > 0 || record.proved.len() != old.proved.len() {
        write(repo, &record)?;
    }
    Ok(ran)
}

/// Runs one invocation with its output kept, never echoed: whether it
/// passed (None when it couldn't start for want of its tool), the seconds
/// it took, and why it didn't pass.
fn run_one(root: &Path, inv: &Invocation, limit: Duration) -> (Option<bool>, f64, Option<String>) {
    let Some((program, args)) = inv.argv.split_first() else {
        return (Some(false), 0.0, Some("it has no command".into()));
    };
    let mut cmd = fairlead_core::process::command(program, &root.join(&inv.cwd));
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (None, 0.0, Some(format!("`{program}` isn't installed")));
        }
        Err(e) => {
            return (
                Some(false),
                0.0,
                Some(format!("could not start: {program}: {e}")),
            )
        }
    };
    let log = Arc::new(Mutex::new(Vec::new()));
    let (done, drained) = mpsc::channel();
    let streams: [Option<Box<dyn std::io::Read + Send>>; 2] = [
        child.stdout.take().map(|s| Box::new(s) as _),
        child.stderr.take().map(|s| Box::new(s) as _),
    ];
    for mut from in streams.into_iter().flatten() {
        let (log, done) = (log.clone(), done.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = from.read(&mut buf) {
                if n == 0 {
                    break;
                }
                log.lock().expect("log lock").extend_from_slice(&buf[..n]);
            }
            let _ = done.send(());
        });
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < limit => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    // A process the runner left behind can hold the pipes open, so the
    // readers get a moment to finish and are never waited on for good.
    for _ in 0..2 {
        let _ = drained.recv_timeout(Duration::from_secs(1));
    }
    let text = String::from_utf8_lossy(&log.lock().expect("log lock")).into_owned();
    match status {
        None => {
            let why = format!("no result within {} s", limit.as_secs());
            (Some(false), seconds, Some(why))
        }
        Some(status) if status.success() => (Some(true), seconds, None),
        // What a shell exits with when the command it was given isn't there.
        Some(status) if status.code() == Some(127) => (
            None,
            seconds,
            Some(format!("`{program}` couldn't find its tool (exit 127)")),
        ),
        Some(status) => {
            let why = crate::step::why_failed(inv, &text, status.code());
            (Some(false), seconds, Some(why))
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use fairlead_core::plan::InvocationKind;

    fn inv(argv: &[&str]) -> Invocation {
        Invocation {
            id: "unit".into(),
            kind: InvocationKind::Runner,
            cwd: ".".into(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            quarantined: None,
        }
    }

    #[test]
    fn a_run_passes_fails_with_its_reason_or_cannot_start_for_want_of_its_tool() {
        let here = Path::new(".");
        let (passed, _, why) = run_one(here, &inv(&["sh", "-c", "echo 1 passed"]), LIMIT);
        assert_eq!((passed, why), (Some(true), None));
        let failing = inv(&[
            "sh",
            "-c",
            "echo '1 failed'; echo 'error: boom' >&2; exit 1",
        ]);
        let (passed, _, why) = run_one(here, &failing, LIMIT);
        assert_eq!(passed, Some(false));
        assert!(why.as_deref().unwrap().contains("error: boom"), "{why:?}");
        let (passed, _, why) = run_one(here, &inv(&["no-such-tool-x", "a.test.ts"]), LIMIT);
        assert_eq!(
            (passed, why.as_deref()),
            (None, Some("`no-such-tool-x` isn't installed"))
        );
        let (passed, _, _) = run_one(here, &inv(&["sh", "-c", "no-such-tool-x"]), LIMIT);
        assert_eq!(passed, None, "a shell's exit 127 is a missing tool");
    }

    #[test]
    fn a_run_past_the_limit_is_stopped_and_counts_as_failed() {
        let started = Instant::now();
        let slow = inv(&["sleep", "30"]);
        let (passed, seconds, why) = run_one(Path::new("."), &slow, Duration::from_millis(200));
        assert_eq!(passed, Some(false));
        assert_eq!(why.as_deref(), Some("no result within 0 s"));
        assert!(seconds < 5.0 && started.elapsed() < Duration::from_secs(10));
    }
}
