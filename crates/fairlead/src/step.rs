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
    let (outcome, log, _) = run_saying_why(who, root, inv, held, keep);
    (outcome, log)
}

/// `run`, with why a failed step failed when it did (see `why_failed`).
pub fn run_saying_why(
    who: &str,
    root: &Path,
    inv: &Invocation,
    held: Option<&Quarantined>,
    keep: bool,
) -> (Outcome, String, Option<String>) {
    let Some((program, args)) = inv.argv.split_first() else {
        eprintln!("{who}: {} has no command", inv.id);
        return (
            Outcome::Failed,
            String::new(),
            Some("it has no command".into()),
        );
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
            let why = format!("could not start: {program}: {e}");
            return (Outcome::Failed, String::new(), Some(why));
        }
    };
    if status.success() {
        return (Outcome::Passed, log, None);
    }
    let why = Some(why_failed(inv, &log, status.code()));
    let Some(q) = held else {
        return (Outcome::Failed, log, why);
    };
    match excused(q, inv, &log, &Here::detect(root)) {
        Ok(()) => {
            eprintln!(
                "{who}: {} failed as its [[quarantine]] entry expects, so it isn't provable here: {}",
                q.target,
                quarantine::line(q)
            );
            (Outcome::Held, log, None)
        }
        Err(counts) => {
            eprintln!(
                "{who}: {} failed, and the failure counts: {counts}",
                q.target
            );
            (Outcome::Failed, log, why)
        }
    }
}

/// Why a step that exited non-zero failed, in a few words, from its output:
/// a runner whose output shows no test result couldn't start, one that
/// shows them failed so many tests, and either way the first line that
/// reads as an error.
pub fn why_failed(inv: &Invocation, log: &str, code: Option<i32>) -> String {
    let exit = code.map_or_else(|| "killed by a signal".to_string(), |c| format!("exit {c}"));
    let plain = ansi().replace_all(log, "");
    let lines: Vec<&str> = plain
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let error = lines
        .iter()
        .find(|l| {
            let l = l.to_lowercase();
            [
                "error",
                "refused",
                "not found",
                "enoent",
                "cannot",
                "can't",
                "no such",
            ]
            .iter()
            .any(|w| l.contains(w))
        })
        .or(lines.last())
        .map(|l| {
            let cut: String = l.chars().take(160).collect();
            format!(": {cut}")
        })
        .unwrap_or_else(|| ", with no output".to_string());
    if inv.kind == InvocationKind::Check {
        return format!("{exit}{error}");
    }
    if !results().is_match(&plain) {
        return format!("could not start ({exit} before any test ran){error}");
    }
    let failed = extractor_for(&inv.argv).map_or(0, |e| {
        let mut tests: Vec<_> = extract(&e, log)
            .into_iter()
            .map(|p| (p.path, p.title))
            .collect();
        tests.sort();
        tests.dedup();
        tests.len()
    });
    match failed {
        0 => format!("{exit} after its tests ran{error}"),
        1 => format!("1 test failed ({exit})"),
        n => format!("{n} tests failed ({exit})"),
    }
}

fn ansi() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").expect("valid"))
}

/// A line any common runner prints once a test has run: a count of passed
/// or failed tests, TAP's `ok N`, Go's `--- PASS`, or a check mark.
fn results() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?im)\b\d+\s+(pass|passed|passing|fail|failed|failing|tests?|specs?|examples?)\b|^\s*(not )?ok \d|^\s*--- (PASS|FAIL)|^\s*(PASS|FAIL)\s|[✓✔✗✘×]",
        )
        .expect("valid")
    })
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

    #[test]
    fn a_failure_says_whether_any_test_ran_and_its_first_error_line() {
        let run = |argv: &[&str]| Invocation {
            id: "api".into(),
            kind: InvocationKind::Runner,
            cwd: ".".into(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            quarantined: None,
        };
        let boot =
            "\x1b[31m$ bun test api/a.test.ts\x1b[0m\nerror: connect ECONNREFUSED 127.0.0.1:5432\n";
        assert_eq!(
            why_failed(&run(&["bun", "test"]), boot, Some(1)),
            "could not start (exit 1 before any test ran): error: connect ECONNREFUSED 127.0.0.1:5432"
        );
        let after = "api/a.test.ts:\n(pass) a > b\n 3 pass\n 0 fail\nerror: coverage below 80%\n";
        assert_eq!(
            why_failed(&run(&["./run-api"]), after, Some(2)),
            "exit 2 after its tests ran: error: coverage below 80%"
        );
        assert_eq!(
            why_failed(&run(&["./run-api"]), "", None),
            "could not start (killed by a signal before any test ran), with no output"
        );
        let check = Invocation {
            kind: InvocationKind::Check,
            ..run(&["tsc"])
        };
        assert_eq!(
            why_failed(&check, "src/a.ts(1,7): error TS2322: no\n", Some(2)),
            "exit 2: src/a.ts(1,7): error TS2322: no"
        );
    }
}
