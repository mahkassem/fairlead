//! `fairlead test bench`: which of Bun's test modes is fastest on the
//! repository's own tests, and how `bun check` compares with its `tsc` check.
//! Each mode runs more than once, since a file that passes in one run and
//! fails in another depends on what ran beside it, and a mode that trips
//! such a file isn't recommended however fast it is. The last result is kept
//! under `.fairlead/` for `doctor --score`, `insights --suggest` and `migrate`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{ExitCode, Stdio};
use std::time::{Instant, SystemTime};

use clap::{Args, Subcommand};
use fairlead_core::config::{self, Config, Invoke, LoadOptions, Runner, TestClass};
use serde::{Deserialize, Serialize};

mod argv;
use argv::{bun_check_argv, judge, mode_argv, relative, runs_tsc};

/// Where the last result is kept, from the repository root.
pub const RESULT: &str = ".fairlead/test-bench.json";
/// The first Bun with `bun test --parallel` and `bun check`.
const PARALLEL_SINCE: [u64; 3] = [1, 4, 4];

#[derive(Subcommand)]
pub enum TestAction {
    /// Time `bun test` serial, `--parallel` and `--parallel --no-isolate` on
    /// the repository's tests, and `bun check` against a `tsc` check, then
    /// recommend the fastest mode no order-dependent test fails in.
    Bench(BenchArgs),
}

#[derive(Args)]
pub struct BenchArgs {
    /// The runner to measure; the first whose command is `bun test` by default.
    #[arg(long, value_name = "ID")]
    runner: Option<String>,
    /// How many times each mode runs; a file that fails in one run and passes in another is order-dependent.
    #[arg(long, value_name = "N", default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..=10))]
    runs: u32,
    /// Print the result as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Serial,
    Parallel,
    ParallelNoIsolate,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Serial, Mode::Parallel, Mode::ParallelNoIsolate];

    fn flags(self) -> &'static [&'static str] {
        match self {
            Mode::Serial => &[],
            Mode::Parallel => &["--parallel"],
            Mode::ParallelNoIsolate => &["--parallel", "--no-isolate"],
        }
    }

    /// The mode as a person would type it.
    pub fn label(self) -> String {
        let mut words = vec!["bun", "test"];
        words.extend(self.flags());
        words.join(" ")
    }

    /// The mode a runner's command runs in.
    pub fn of(command: &[String]) -> Mode {
        let parallel = command.iter().any(|a| is_parallel(a));
        match (parallel, command.iter().any(|a| a == "--no-isolate")) {
            (false, _) => Mode::Serial,
            (true, false) => Mode::Parallel,
            (true, true) => Mode::ParallelNoIsolate,
        }
    }
}

fn is_parallel(arg: &str) -> bool {
    arg == "--parallel" || arg.starts_with("--parallel=")
}

fn is_mode_flag(arg: &str) -> bool {
    is_parallel(arg) || arg == "--isolate" || arg == "--no-isolate"
}

/// Whether a runner's command is `bun test`, which the benchmark measures.
pub fn is_bun_test(command: &[String]) -> bool {
    let program = command
        .first()
        .and_then(|p| Path::new(p).file_stem())
        .and_then(|s| s.to_str());
    program == Some("bun") && command.get(1).is_some_and(|a| a == "test")
}

/// `command` in `mode`: its own mode flags dropped and the mode's put right
/// after `test`, keeping a worker count it gave `--parallel`.
pub fn with_mode(command: &[String], mode: Mode) -> Vec<String> {
    let count = command.iter().find(|a| a.starts_with("--parallel="));
    let mut out: Vec<String> = command
        .iter()
        .filter(|a| !is_mode_flag(a))
        .cloned()
        .collect();
    let at = out.len().min(2);
    for (i, flag) in mode.flags().iter().enumerate() {
        let flag = match count {
            Some(c) if *flag == "--parallel" => c.clone(),
            _ => flag.to_string(),
        };
        out.insert(at + i, flag);
    }
    out
}

/// Whether Bun `version` has `--parallel`; a version that doesn't parse doesn't.
pub fn parallel_available(version: &str) -> bool {
    let numbers: Vec<u64> = version
        .trim()
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .take(3)
        .map_while(|p| p.parse().ok())
        .collect();
    numbers.len() == 3 && numbers.as_slice() >= PARALLEL_SINCE.as_slice()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Run {
    pub seconds: f64,
    /// Whether the run wrote its JUnit report; without one no file is judged.
    pub reported: bool,
    pub passed: usize,
    /// Files that failed, or that the report leaves out, as one that fails to load.
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Timed {
    pub mode: Mode,
    pub runs: Vec<Run>,
    pub mean_seconds: f64,
}

impl Timed {
    pub fn new(mode: Mode, runs: Vec<Run>) -> Timed {
        let mean_seconds = runs.iter().map(|r| r.seconds).sum::<f64>() / runs.len().max(1) as f64;
        Timed {
            mode,
            runs,
            mean_seconds,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderDependent {
    pub file: String,
    /// The runs it failed in, as `bun test --parallel, run 2`.
    pub failed_in: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timing {
    pub seconds: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Typecheck {
    pub check: String,
    pub tsc: Vec<String>,
    pub tsc_runs: Vec<Timing>,
    pub bun_check: Vec<String>,
    pub bun_check_runs: Vec<Timing>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bench {
    pub at: String,
    pub bun: String,
    pub runner: String,
    /// The runner's command when it was measured.
    pub command: Vec<String>,
    pub configured: Mode,
    pub files: usize,
    pub modes: Vec<Timed>,
    pub order_dependent: Vec<OrderDependent>,
    pub recommended: Option<Mode>,
    pub reason: String,
    pub typecheck: Option<Typecheck>,
}

fn run_name(mode: Mode, run: usize) -> String {
    format!("{}, run {}", mode.label(), run + 1)
}

/// Files that failed in one run and passed in another, by any mode. A file
/// that fails in every judged run is broken, not order-dependent.
pub fn order_dependent(modes: &[Timed]) -> Vec<OrderDependent> {
    let mut failed: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut judged = 0;
    for timed in modes {
        for (i, run) in timed.runs.iter().enumerate().filter(|(_, r)| r.reported) {
            judged += 1;
            for file in &run.failed {
                failed
                    .entry(file)
                    .or_default()
                    .push(run_name(timed.mode, i));
            }
        }
    }
    failed
        .into_iter()
        .filter(|(_, runs)| runs.len() < judged)
        .map(|(file, failed_in)| OrderDependent {
            file: file.to_string(),
            failed_in,
        })
        .collect()
}

/// Why `timed` can't be recommended, if it can't.
fn disqualified(timed: &Timed, order: &[OrderDependent]) -> Option<String> {
    if timed.runs.iter().any(|r| !r.reported) {
        return Some("a run wrote no report".into());
    }
    let tripped: Vec<&str> = order
        .iter()
        .filter(|o| {
            o.failed_in
                .iter()
                .any(|r| r.starts_with(&format!("{},", timed.mode.label())))
        })
        .map(|o| o.file.as_str())
        .collect();
    match tripped.as_slice() {
        [] => None,
        [one] => Some(format!("{one} failed in it")),
        [one, rest @ ..] => Some(format!("{one} and {} more failed in it", rest.len())),
    }
}

/// The fastest mode no order-dependent file failed in, and why, or none.
pub fn recommend(bun: &str, modes: &[Timed], order: &[OrderDependent]) -> (Option<Mode>, String) {
    if !parallel_available(bun) {
        return (
            None,
            format!("Bun {bun} has no `bun test --parallel`, which came in Bun 1.4.4"),
        );
    }
    let mut by_speed: Vec<&Timed> = modes.iter().collect();
    by_speed.sort_by(|a, b| a.mean_seconds.total_cmp(&b.mean_seconds));
    let mut passed_over = Vec::new();
    for timed in by_speed {
        match disqualified(timed, order) {
            Some(why) => passed_over.push(format!(
                "{} ({}): {why}",
                timed.mode.label(),
                secs(timed.mean_seconds)
            )),
            None => {
                let mut reason = format!(
                    "the fastest mode with no order-dependent failure ({})",
                    secs(timed.mean_seconds)
                );
                if !passed_over.is_empty() {
                    reason.push_str(&format!("; faster, but {}", passed_over.join("; ")));
                }
                return (Some(timed.mode), reason);
            }
        }
    }
    (
        None,
        format!(
            "no mode ran clean: {}; fix the order-dependent tests and run it again",
            passed_over.join("; ")
        ),
    )
}

pub fn secs(s: f64) -> String {
    format!("{s:.2} s")
}

/// The modes' mean times in one line, as a suggestion quotes them.
pub fn summary(b: &Bench) -> String {
    let times: Vec<String> = b
        .modes
        .iter()
        .map(|t| format!("{} {}", t.mode.label(), secs(t.mean_seconds)))
        .collect();
    let runs = b.modes.first().map_or(0, |t| t.runs.len());
    format!(
        "`fairlead test bench` on {} ({} test files, Bun {}, mean of {runs} run(s)): {}",
        b.at.get(..10).unwrap_or(&b.at),
        b.files,
        b.bun,
        times.join(", ")
    )
}

fn timed(argv: &[String], cwd: &Path) -> Result<Timing, String> {
    let (program, args) = argv.split_first().ok_or("an empty command")?;
    let mut cmd = fairlead_core::process::command(program, cwd);
    cmd.args(args).stdout(Stdio::null()).stderr(Stdio::null());
    let start = Instant::now();
    let status = cmd
        .status()
        .map_err(|e| format!("couldn't start {program}: {e}"))?;
    Ok(Timing {
        seconds: start.elapsed().as_secs_f64(),
        passed: status.success(),
    })
}

fn bun_version(bun: &str, cwd: &Path) -> Result<String, String> {
    let out = fairlead_core::process::command(bun, cwd)
        .arg("--version")
        .output()
        .map_err(|e| format!("couldn't run `{bun} --version`: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.is_empty() {
        return Err(format!("`{bun} --version` printed no version"));
    }
    Ok(text)
}

/// The runner to measure and the test files it runs whenever everything
/// does: every class but `demand`, relative to the runner's directory.
fn target(
    root: &Path,
    config: &Config,
    wanted: Option<&str>,
) -> Result<(Runner, Vec<String>), String> {
    let runners = config.tests.runners.items();
    let index = match wanted {
        Some(id) => runners
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| format!("no runner `{id}` in the config"))?,
        None => runners
            .iter()
            .position(|r| is_bun_test(&r.command))
            .ok_or("no runner's command is `bun test`; the bench measures Bun's test modes")?,
    };
    let runner = runners[index].clone();
    if !is_bun_test(&runner.command) {
        return Err(format!(
            "runner `{}` doesn't run `bun test`; the bench measures Bun's test modes",
            runner.id
        ));
    }
    let cwd = runner.cwd.clone().unwrap_or_else(|| ".".into());
    if runner.invoke == Invoke::PerModule || cwd.contains('{') {
        return Err(format!(
            "runner `{}` runs once per module; the bench measures a runner with one working directory",
            runner.id
        ));
    }
    let tree = fairlead_lang::tree::Tree::scan(root);
    let found = fairlead_tests::testfiles::discover(
        &tree,
        config,
        &fairlead_tests::modules::Modules::default(),
    )?;
    let files: Vec<String> = found
        .tests
        .iter()
        .filter(|t| t.runner == Some(index) && t.class != TestClass::Demand)
        .map(|t| relative(&cwd, &t.path))
        .collect();
    if files.is_empty() {
        return Err(format!("runner `{}` has no test files to time", runner.id));
    }
    Ok((runner, files))
}

fn measure_modes(
    runner: &Runner,
    files: &[String],
    dir: &Path,
    runs: u32,
) -> Result<Vec<Timed>, String> {
    let report =
        std::env::temp_dir().join(format!("fairlead-test-bench-{}.xml", std::process::id()));
    let cwd = runner.cwd.as_deref().unwrap_or(".");
    let mut out = Vec::new();
    for mode in Mode::ALL {
        let argv = mode_argv(&runner.command, mode, &report, files);
        let mut done = Vec::new();
        for run in 0..runs {
            eprintln!("test bench: {}, run {} of {runs}", mode.label(), run + 1);
            let _ = std::fs::remove_file(&report);
            let timing = timed(&argv, dir)?;
            let text = std::fs::read_to_string(&report).ok();
            let (passed, failed) = judge(files, text.as_deref(), cwd);
            done.push(Run {
                seconds: timing.seconds,
                reported: text.is_some(),
                passed,
                failed,
            });
        }
        out.push(Timed::new(mode, done));
    }
    let _ = std::fs::remove_file(&report);
    Ok(out)
}

fn measure_typecheck(
    config: &Config,
    bun: &str,
    root: &Path,
    runs: u32,
) -> Result<Option<Typecheck>, String> {
    let Some((check, at)) = config
        .checks
        .items()
        .iter()
        .find_map(|c| runs_tsc(&c.command).map(|i| (c, i)))
    else {
        return Ok(None);
    };
    let tsc: Vec<String> = check
        .command
        .iter()
        .filter(|a| *a != "{files}")
        .cloned()
        .collect();
    let bun_check = bun_check_argv(bun, &check.command[at + 1..]);
    let mut tsc_runs = Vec::new();
    let mut bun_check_runs = Vec::new();
    for run in 0..runs {
        eprintln!(
            "test bench: check `{}`, run {} of {runs}",
            check.id,
            run + 1
        );
        tsc_runs.push(timed(&tsc, root)?);
        bun_check_runs.push(timed(&bun_check, root)?);
    }
    Ok(Some(Typecheck {
        check: check.id.clone(),
        tsc,
        tsc_runs,
        bun_check,
        bun_check_runs,
    }))
}

fn mean(runs: &[Timing]) -> f64 {
    runs.iter().map(|t| t.seconds).sum::<f64>() / runs.len().max(1) as f64
}

pub fn render(b: &Bench) -> String {
    let mut out = format!(
        "test bench: runner `{}`, {} test files, Bun {}\n",
        b.runner, b.files, b.bun
    );
    let width = Mode::ALL.iter().map(|m| m.label().len()).max().unwrap_or(0);
    for t in &b.modes {
        let runs: Vec<String> = t.runs.iter().map(|r| secs(r.seconds)).collect();
        let failed: usize = t.runs.iter().map(|r| r.failed.len()).sum();
        let mut line = format!(
            "  {:<width$}  mean {:>9}  runs {}",
            t.mode.label(),
            secs(t.mean_seconds),
            runs.join(", ")
        );
        if t.runs.iter().any(|r| !r.reported) {
            line.push_str("  (a run wrote no report)");
        } else if failed > 0 {
            line.push_str(&format!("  ({failed} file failure(s))"));
        }
        if t.mode == b.configured {
            line.push_str("  [configured]");
        }
        out.push_str(&format!("{line}\n"));
    }
    for o in &b.order_dependent {
        out.push_str(&format!(
            "order-dependent: {} failed in {}, and passed in the other runs\n",
            o.file,
            o.failed_in.join("; ")
        ));
    }
    match b.recommended {
        Some(m) if m == b.configured => out.push_str(&format!(
            "recommend: {}, which the runner already runs: {}\n",
            m.label(),
            b.reason
        )),
        Some(m) => out.push_str(&format!(
            "recommend: {}, {}; the runner runs {}\n  command = {:?}\n",
            m.label(),
            b.reason,
            b.configured.label(),
            with_mode(&b.command, m)
        )),
        None => out.push_str(&format!("recommend: nothing; {}\n", b.reason)),
    }
    if let Some(t) = &b.typecheck {
        let state = |runs: &[Timing]| {
            if runs.iter().all(|r| r.passed) {
                "passed"
            } else {
                "failed"
            }
        };
        out.push_str(&format!(
            "typecheck: check `{}` ({}) mean {}, {}; `{}` mean {}, {}\n",
            t.check,
            t.tsc.join(" "),
            secs(mean(&t.tsc_runs)),
            state(&t.tsc_runs),
            t.bun_check.join(" "),
            secs(mean(&t.bun_check_runs)),
            state(&t.bun_check_runs)
        ));
    }
    out
}

/// The last result, if one was written and still reads.
pub fn load(root: &Path) -> Option<Bench> {
    let text = std::fs::read_to_string(root.join(RESULT)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save(root: &Path, b: &Bench) -> Result<(), String> {
    let path = root.join(RESULT);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(b).expect("the result prints");
    std::fs::write(&path, format!("{text}\n")).map_err(|e| format!("{}: {e}", path.display()))
}

fn bench(args: &BenchArgs, cwd: &Path) -> Result<Bench, String> {
    let loaded =
        config::load(cwd, &LoadOptions::from_process(Vec::new())).map_err(|e| e.to_string())?;
    if let Some(p) = loaded.problems.first() {
        return Err(format!(
            "the config has problems, such as {}: {} (see `fairlead config check`)",
            p.key, p.message
        ));
    }
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    let (runner, files) = target(&root, &loaded.config, args.runner.as_deref())?;
    let dir = root.join(runner.cwd.as_deref().unwrap_or("."));
    let program = runner.command[0].clone();
    let version = bun_version(&program, &dir)?;
    let mut b = Bench {
        at: fairlead_guard::events::timestamp(SystemTime::now()),
        bun: version.clone(),
        runner: runner.id.clone(),
        command: runner.command.clone(),
        configured: Mode::of(&runner.command),
        files: files.len(),
        modes: Vec::new(),
        order_dependent: Vec::new(),
        recommended: None,
        reason: String::new(),
        typecheck: None,
    };
    if parallel_available(&version) {
        b.modes = measure_modes(&runner, &files, &dir, args.runs)?;
        b.order_dependent = order_dependent(&b.modes);
        b.typecheck = measure_typecheck(&loaded.config, &program, &root, args.runs)?;
    }
    (b.recommended, b.reason) = recommend(&version, &b.modes, &b.order_dependent);
    save(&root, &b)?;
    Ok(b)
}

pub fn run(args: BenchArgs, cwd: &Path) -> ExitCode {
    match bench(&args, cwd) {
        Ok(b) => {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&b).expect("the result prints")
                );
            } else {
                print!("{}", render(&b));
                println!("wrote {RESULT}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("test bench: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn run(seconds: f64, failed: &[&str]) -> Run {
        Run {
            seconds,
            reported: true,
            passed: 3 - failed.len(),
            failed: argv(failed),
        }
    }

    fn timed(mode: Mode, runs: Vec<Run>) -> Timed {
        Timed::new(mode, runs)
    }

    #[test]
    fn a_mode_is_read_from_the_command_and_written_back_into_it() {
        let plain = argv(&["bun", "test", "{files}"]);
        assert_eq!(Mode::of(&plain), Mode::Serial);
        let fast = with_mode(&plain, Mode::ParallelNoIsolate);
        assert_eq!(
            fast,
            argv(&["bun", "test", "--parallel", "--no-isolate", "{files}"])
        );
        assert_eq!(Mode::of(&fast), Mode::ParallelNoIsolate);
        assert_eq!(with_mode(&fast, Mode::Serial), plain);
        let counted = argv(&["bun", "test", "--parallel=4", "{files}"]);
        assert_eq!(Mode::of(&counted), Mode::Parallel);
        assert_eq!(
            with_mode(&counted, Mode::ParallelNoIsolate),
            argv(&["bun", "test", "--parallel=4", "--no-isolate", "{files}"]),
            "a worker count stays"
        );
        assert!(is_bun_test(&argv(&["/usr/local/bin/bun", "test"])));
        assert!(!is_bun_test(&argv(&["bunx", "vitest"])));
        assert!(!is_bun_test(&argv(&["bun", "run", "test"])));
    }

    #[test]
    fn parallel_starts_at_bun_1_4_4() {
        assert!(parallel_available("1.4.4"));
        assert!(parallel_available("1.10.0"));
        assert!(parallel_available("1.4.5-canary.1+abc"));
        assert!(!parallel_available("1.4.3"));
        assert!(!parallel_available("1.3.12"));
        assert!(!parallel_available("nonsense"));
    }

    #[test]
    fn a_file_that_fails_in_some_runs_is_order_dependent_and_one_failing_in_all_is_not() {
        let modes = [
            timed(
                Mode::Serial,
                vec![run(3.0, &["broken"]), run(3.0, &["broken"])],
            ),
            timed(
                Mode::Parallel,
                vec![run(1.0, &["broken", "leaky"]), run(1.0, &["broken"])],
            ),
        ];
        let found = order_dependent(&modes);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].file, "leaky");
        assert_eq!(found[0].failed_in, vec!["bun test --parallel, run 1"]);
    }

    #[test]
    fn the_fastest_mode_wins_unless_an_order_dependent_file_failed_in_it() {
        let modes = vec![
            timed(Mode::Serial, vec![run(3.0, &[]), run(3.2, &[])]),
            timed(Mode::Parallel, vec![run(0.5, &["a"]), run(0.5, &[])]),
            timed(Mode::ParallelNoIsolate, vec![run(1.0, &[]), run(1.2, &[])]),
        ];
        let order = order_dependent(&modes);
        let (mode, why) = recommend("1.4.4", &modes, &order);
        assert_eq!(mode, Some(Mode::ParallelNoIsolate));
        assert!(why.contains("1.10 s"), "{why}");
        assert!(
            why.contains("faster, but bun test --parallel (0.50 s): a failed in it"),
            "{why}"
        );
        let clean: Vec<Timed> = modes
            .iter()
            .cloned()
            .map(|mut t| {
                t.runs.iter_mut().for_each(|r| r.failed.clear());
                t
            })
            .collect();
        assert_eq!(recommend("1.4.4", &clean, &[]).0, Some(Mode::Parallel));
    }

    #[test]
    fn nothing_is_recommended_before_bun_1_4_4_or_when_every_mode_tripped() {
        let (mode, why) = recommend("1.3.0", &[], &[]);
        assert_eq!(mode, None);
        assert!(why.contains("came in Bun 1.4.4"), "{why}");
        let modes = vec![
            timed(Mode::Serial, vec![run(3.0, &["a"]), run(3.0, &[])]),
            timed(
                Mode::Parallel,
                vec![Run {
                    reported: false,
                    ..run(1.0, &[])
                }],
            ),
        ];
        let (mode, why) = recommend("1.4.4", &modes, &order_dependent(&modes));
        assert_eq!(mode, None);
        assert!(
            why.contains("wrote no report") && why.contains("a failed in it"),
            "{why}"
        );
    }
}
