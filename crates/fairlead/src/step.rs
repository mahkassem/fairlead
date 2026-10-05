//! Running one planned invocation, as `fairlead done` and `ci run` do, and
//! judging a failure that a `[[quarantine]]` entry holding it expects.

use std::path::Path;
use std::process::{Command, ExitStatus};

use fairlead_core::plan::{Invocation, InvocationKind, Quarantined};
use fairlead_replay::extract::{extract, Extractor};
use fairlead_tests::quarantine::{self, Here};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    /// It failed as its `[[quarantine]]` entry expects: not provable here.
    Held,
}

/// Runs `inv` in `root`, its output echoed as it comes. The output is kept
/// and returned when `keep` is set, or when an entry holds the invocation,
/// since a failure is then read for the entry's signature. Every line the
/// run prints about itself starts with `who`.
pub fn run(
    who: &str,
    root: &Path,
    inv: &Invocation,
    held: Option<&Quarantined>,
    keep: bool,
) -> (Outcome, String) {
    let Some((program, args)) = inv.argv.split_first() else {
        eprintln!("{who}: {} has no command", inv.id);
        return (Outcome::Failed, String::new());
    };
    let mut cmd = fairlead_core::process::command(program, &root.join(&inv.cwd));
    cmd.args(args);
    let result = if keep || held.is_some() {
        captured(&mut cmd)
    } else {
        cmd.status().map(|s| (s, String::new()))
    };
    let (status, log) = match result {
        Ok(done) => done,
        Err(e) => {
            eprintln!("{who}: couldn't start {program}: {e}");
            return (Outcome::Failed, String::new());
        }
    };
    if status.success() {
        return (Outcome::Passed, log);
    }
    let Some(q) = held else {
        return (Outcome::Failed, log);
    };
    match excused(q, inv, &log, &Here::detect(root)) {
        Ok(()) => {
            eprintln!(
                "{who}: {} failed as its [[quarantine]] entry expects, so it isn't provable here: {}",
                q.target,
                quarantine::line(q)
            );
            (Outcome::Held, log)
        }
        Err(why) => {
            eprintln!("{who}: {} failed, and the failure counts: {why}", q.target);
            (Outcome::Failed, log)
        }
    }
}

/// Whether a failed invocation is the one failure `q` expects on the machine
/// running it, or why it counts. The plan may come from another machine, so
/// its OS and conditions are checked again here, and a held file whose
/// output names more than one failed test counts: the entry excuses one.
fn excused(q: &Quarantined, inv: &Invocation, log: &str, here: &Here) -> Result<(), String> {
    for reason in &q.here {
        if reason != here.os && !here.conditions.iter().any(|c| c.name() == reason) {
            return Err(format!(
                "its [[quarantine]] entry holds on {}, and this is {}",
                q.here.join(", "),
                here.describe()
            ));
        }
    }
    if q.until.as_str() < here.today.as_str() {
        return Err(format!("its [[quarantine]] entry ended on {}", q.until));
    }
    if !quarantine::expected(q, log, &here.today) {
        return Err(format!(
            "the output doesn't match its [[quarantine]] signature `{}`",
            q.signature
        ));
    }
    if inv.kind == InvocationKind::Runner {
        if let Some(extractor) = extractor_for(&inv.argv) {
            let mut failed: Vec<_> = extract(&extractor, log)
                .into_iter()
                .map(|p| (p.path, p.title))
                .collect();
            failed.sort();
            failed.dedup();
            if failed.len() > 1 {
                return Err(format!(
                    "{} tests failed in it, and its [[quarantine]] entry excuses only the one it expects",
                    failed.len()
                ));
            }
        }
    }
    Ok(())
}

/// The extractor that reads this runner's output, judged from its command.
fn extractor_for(argv: &[String]) -> Option<Extractor> {
    let stem = |a: &String| {
        Path::new(a)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    };
    let words: Vec<String> = argv.iter().map(stem).collect();
    let has = |w: &str| words.iter().any(|x| x == w);
    let name = if has("vitest") {
        "vitest"
    } else if has("jest") {
        "jest"
    } else if has("pytest") {
        "pytest"
    } else if has("pest") {
        "pest"
    } else if has("phpunit") {
        "phpunit"
    } else if words.first().is_some_and(|w| w == "bun") && has("test") {
        "bun"
    } else if words.first().is_some_and(|w| w == "go") && has("test") {
        "go"
    } else {
        return None;
    };
    Extractor::named(name, None).ok()
}

/// Runs a command with its output echoed as it comes and kept, so a failing
/// run's report can be read.
fn captured(cmd: &mut Command) -> std::io::Result<(ExitStatus, String)> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let pump = |mut from: Box<dyn Read + Send>, err: bool| {
        let log = log.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = from.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let _ = if err {
                    std::io::stderr().write_all(&buf[..n])
                } else {
                    std::io::stdout().write_all(&buf[..n])
                };
                log.lock().expect("log lock").extend_from_slice(&buf[..n]);
            }
        })
    };
    let out = pump(Box::new(child.stdout.take().expect("piped stdout")), false);
    let err = pump(Box::new(child.stderr.take().expect("piped stderr")), true);
    let status = child.wait()?;
    let _ = out.join();
    let _ = err.join();
    let text = String::from_utf8_lossy(&log.lock().expect("log lock")).into_owned();
    Ok((status, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fairlead_core::config::Condition;

    fn held() -> Quarantined {
        Quarantined {
            target: "test/tracked.test.ts".into(),
            kind: InvocationKind::Runner,
            here: vec!["windows".into(), "space-in-path".into()],
            signature: "spawnSync git ENOENT".into(),
            reason: "it runs git in a file URL's pathname".into(),
            proved_in: "CI on Linux".into(),
            until: "2099-12-31".into(),
        }
    }

    fn vitest_run() -> Invocation {
        Invocation {
            id: "vitest".into(),
            kind: InvocationKind::Runner,
            cwd: ".".into(),
            argv: ["npx", "vitest", "run", "test/tracked.test.ts"]
                .map(String::from)
                .to_vec(),
            quarantined: Some("test/tracked.test.ts".into()),
        }
    }

    fn windows(conditions: Vec<Condition>) -> Here {
        Here {
            os: "windows",
            conditions,
            today: "2026-10-05".into(),
        }
    }

    const ONE: &str =
        " FAIL  test/tracked.test.ts > lists tracked files\nError: spawnSync git ENOENT\n";
    const TWO: &str = " FAIL  test/tracked.test.ts > lists tracked files\nError: spawnSync git ENOENT\n FAIL  test/tracked.test.ts > counts lines\nAssertionError: expected 3 to be 4\n";

    #[test]
    fn one_failed_test_with_the_signature_is_excused() {
        let here = windows(vec![Condition::SpaceInPath]);
        assert_eq!(excused(&held(), &vitest_run(), ONE, &here), Ok(()));
    }

    #[test]
    fn another_failed_test_in_the_held_file_counts() {
        let here = windows(vec![Condition::SpaceInPath]);
        let why = excused(&held(), &vitest_run(), TWO, &here).unwrap_err();
        assert!(why.contains("2 tests failed"), "{why}");
    }

    #[test]
    fn a_plan_made_where_the_entry_holds_excuses_nothing_elsewhere() {
        let linux = Here {
            os: "linux",
            ..windows(vec![Condition::SpaceInPath])
        };
        let why = excused(&held(), &vitest_run(), ONE, &linux).unwrap_err();
        assert!(why.contains("this is linux"), "{why}");
        let no_space = windows(vec![]);
        assert!(excused(&held(), &vitest_run(), ONE, &no_space).is_err());
    }

    #[test]
    fn runners_are_read_by_their_own_extractor() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(matches!(
            extractor_for(&argv(&["bun", "x", "vitest", "run"])),
            Some(Extractor::Vitest)
        ));
        assert!(matches!(
            extractor_for(&argv(&["bun", "test", "a.test.ts"])),
            Some(Extractor::Bun)
        ));
        assert!(matches!(
            extractor_for(&argv(&["go", "test", "./..."])),
            Some(Extractor::Go)
        ));
        assert!(extractor_for(&argv(&["./scripts/check"])).is_none());
    }
}
