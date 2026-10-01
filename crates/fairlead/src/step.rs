//! Running one planned invocation, as `fairlead done` and `ci run` do, and
//! judging a failure that a `[[quarantine]]` entry holding it expects.

use std::path::Path;
use std::process::{Command, ExitStatus};

use fairlead_core::plan::{Invocation, Quarantined};
use fairlead_tests::quarantine;

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
    let today = fairlead_core::coverage::today();
    if quarantine::expected(q, &log, &today) {
        eprintln!(
            "{who}: {} failed as its [[quarantine]] entry expects, so it isn't provable here: {}",
            q.target,
            quarantine::line(q)
        );
        return (Outcome::Held, log);
    }
    let why = if q.until.as_str() < today.as_str() {
        format!("its [[quarantine]] entry ended on {}", q.until)
    } else {
        format!(
            "the output doesn't match its [[quarantine]] signature `{}`",
            q.signature
        )
    };
    eprintln!("{who}: {} failed, and the failure counts: {why}", q.target);
    (Outcome::Failed, log)
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
