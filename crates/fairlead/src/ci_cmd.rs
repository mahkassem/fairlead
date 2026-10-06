//! `fairlead ci plan` and `fairlead ci run --plan`: the plan for a CI job,
//! as JSON and GitHub step outputs, and a runner that executes a plan's
//! invocations in order.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Subcommand, ValueEnum};
use fairlead_core::config::LoadOptions;
use fairlead_core::plan::{Plan, VERSION};

use crate::plan_cmd::{make, Changes};
use crate::stage::{self, StageArg};
use crate::step::Outcome;

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Json,
    Github,
}

#[derive(Subcommand)]
pub enum CiAction {
    /// Plan the checked-out commit against a base and write the plan file.
    Plan {
        #[command(flatten)]
        changes: Changes,
        /// The commit CI checked out; the plan refuses to run if HEAD differs.
        #[arg(long, value_name = "SHA")]
        head: Option<String>,
        /// `github` also writes step outputs to $GITHUB_OUTPUT.
        #[arg(long, value_enum, default_value = "json")]
        format: Format,
        /// Where to write the plan JSON. Defaults to `fairlead-plan.json` in
        /// $RUNNER_TEMP, or the system temp directory, never the working tree,
        /// where it would count as a change on the next plan.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// The CI stage to plan for. Without it, a config that sets `[stages]`
        /// or a step's `from` reads the stage from the GitHub event (`auto`),
        /// and any other config plans with no stage.
        #[arg(long, value_enum)]
        stage: Option<StageArg>,
        /// At the merge stage, plan from the head of the newest earlier run of
        /// this workflow on this branch where the job named JOB passed; repeat
        /// for several jobs. When that commit can't be trusted, plan everything.
        #[arg(long, value_name = "JOB")]
        since_green: Vec<String>,
    },
    /// Run a plan's invocations in order; exits non-zero if any fails.
    Run {
        #[arg(long, value_name = "PATH")]
        plan: PathBuf,
        /// Stop at the first failing invocation instead of running the rest.
        #[arg(long)]
        fail_fast: bool,
        /// Write each invocation's outcome and time here, for `ci report`.
        #[arg(long, value_name = "PATH")]
        results: Option<PathBuf>,
        /// The plan the merged pull request ran. Each failing test file is
        /// judged against it: planned, or an escape with the owner rule
        /// that would have caught it.
        #[arg(long, value_name = "PATH")]
        judge: Option<PathBuf>,
        /// Run only these runners and checks, by id.
        #[arg(long, value_name = "ID", conflicts_with = "except")]
        only: Vec<String>,
        /// Run everything but these runners and checks, by id.
        #[arg(long, value_name = "ID")]
        except: Vec<String>,
    },
    /// Write the run's summary: what the plan selected and why, what ran and
    /// failed, and the change's receipt when one is given. To
    /// $GITHUB_STEP_SUMMARY when set, else stdout.
    Report {
        #[arg(long, value_name = "PATH")]
        plan: PathBuf,
        /// The file `ci run --results` wrote.
        #[arg(long, value_name = "PATH")]
        results: Option<PathBuf>,
        /// A receipt the branch carries, as `fairlead receipt --out` wrote it.
        #[arg(long, value_name = "PATH")]
        receipt: Option<PathBuf>,
        /// Also keep one pull request comment up to date with it, over `ci.comment`.
        #[arg(long)]
        comment: bool,
    },
    /// Print a staged GitHub Actions workflow made from the config.
    Workflow {
        /// Write it to `.github/workflows/fairlead.yml` instead, replacing
        /// only a file this command wrote.
        #[arg(long)]
        write: bool,
    },
}

pub fn run(action: CiAction, cwd: &Path) -> ExitCode {
    let result = match action {
        CiAction::Plan {
            changes,
            head,
            format,
            out,
            stage,
            since_green,
        } => {
            let out = out.unwrap_or_else(default_out);
            let green = since_green.as_slice();
            plan(cwd, changes, head.as_deref(), format, &out, stage, green)
        }
        CiAction::Run {
            plan,
            fail_fast,
            results,
            judge,
            only,
            except,
        } => {
            let pick = Pick { only, except };
            execute(
                cwd,
                &plan,
                fail_fast,
                results.as_deref(),
                judge.as_deref(),
                &pick,
            )
        }
        CiAction::Report {
            plan,
            results,
            receipt,
            comment,
        } => crate::ci_report::run(cwd, &plan, results.as_deref(), receipt.as_deref(), comment),
        CiAction::Workflow { write } => crate::ci_workflow::run(cwd, write),
    };
    result.unwrap_or_else(|e| {
        eprintln!("{e}");
        ExitCode::from(2)
    })
}

fn default_out() -> PathBuf {
    std::env::var_os("RUNNER_TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("fairlead-plan.json")
}

fn rev_parse(cwd: &Path, rev: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
        .arg(format!("{rev}^{{commit}}"))
        .output()
        .ok()?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !sha.is_empty()).then_some(sha)
}

fn head_matches(cwd: &Path, expected: &str) -> Result<(), String> {
    let actual = rev_parse(cwd, "HEAD").ok_or("couldn't read HEAD")?;
    let wanted = rev_parse(cwd, expected)
        .ok_or_else(|| format!("--head {expected} isn't a commit in this clone"))?;
    if actual == wanted {
        return Ok(());
    }
    Err(format!(
        "HEAD is {actual}, not {wanted}: check out the head commit before planning, since the plan reads the working tree"
    ))
}

fn plan(
    cwd: &Path,
    mut changes: Changes,
    head: Option<&str>,
    format: Format,
    out: &Path,
    stage_arg: Option<StageArg>,
    since_green: &[String],
) -> Result<ExitCode, String> {
    if let Some(head) = head {
        head_matches(cwd, head)?;
    }
    let config = crate::plan_cmd::config_for(cwd, &changes)?;
    let event = stage::Event::from_env();
    let resolved = stage::resolve(stage_arg, &config, event.as_ref());
    if let Some(s) = resolved.stage {
        println!("stage: {s} ({})", resolved.why);
        if s == fairlead_core::config::CiStage::Full {
            changes.everything = Some("stage full".into());
        }
    }
    let green = crate::since_green::apply(cwd, since_green, resolved.stage, &mut changes);
    let mut planned = make(cwd, &changes)?;
    planned.plan.warnings.extend(green);
    if let Some(s) = resolved.stage {
        stage::apply(&mut planned.plan, &planned.config, s);
    }
    if let Some(line) = crate::reuse::apply(&mut planned.plan, &planned.config, event.as_ref()) {
        println!("{line}");
    }
    let runs = stage::run_outputs(&planned.plan, &planned.config)?;
    let json = serde_json::to_string_pretty(&planned.plan).expect("plan prints");
    let path = cwd.join(out);
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    if format == Format::Github {
        write_outputs(&planned.plan, &path, &runs)?;
    }
    println!(
        "plan {}: {} tests, {} checks, {} invocations{} -> {}",
        planned.plan.plan_id,
        planned.plan.tests.len(),
        planned.plan.checks.len(),
        planned.plan.invocations.len(),
        if planned.plan.all && planned.plan.tests.is_empty() && planned.plan.checks.is_empty() {
            " (everything, and there's nothing to run)"
        } else if planned.plan.all {
            " (everything)"
        } else {
            ""
        },
        path.display()
    );
    for d in &planned.plan.deferred {
        println!(
            "  {} waits for the {} stage ({} selected)",
            d.id, d.from, d.selected
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// The runners and checks `ci run` was asked to run, by id.
struct Pick {
    only: Vec<String>,
    except: Vec<String>,
}

impl Pick {
    fn wants(&self, id: &str) -> bool {
        (self.only.is_empty() || self.only.iter().any(|o| o == id))
            && !self.except.iter().any(|e| e == id)
    }

    /// An id no runner or check has is a typo, not an empty run.
    fn check(&self, cwd: &Path, plan: &Plan) -> Result<(), String> {
        if self.only.is_empty() && self.except.is_empty() {
            return Ok(());
        }
        let config = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
            .map_err(|e| e.to_string())?
            .config;
        let known = |id: &str| {
            config.tests.runners.items().iter().any(|r| r.id == id)
                || config.checks.items().iter().any(|c| c.id == id)
                || plan.invocations.iter().any(|i| i.id == id)
        };
        for id in self.only.iter().chain(&self.except) {
            if !known(id) {
                return Err(format!("no runner or check is called `{id}`"));
            }
        }
        for d in plan.deferred.iter().filter(|d| self.only.contains(&d.id)) {
            println!(
                "fairlead: {} waits for the {} stage; this plan is {}",
                d.id,
                d.from,
                plan.stage.map_or("unstaged", |s| s.name())
            );
        }
        Ok(())
    }
}

/// `name<<DELIM` blocks, so no value can end the block early.
fn output_block(name: &str, value: &str) -> String {
    let mut n = 0u32;
    let mut delim = format!("FAIRLEAD_EOF_{n}");
    while value.contains(&delim) {
        n += 1;
        delim = format!("FAIRLEAD_EOF_{n}");
    }
    format!("{name}<<{delim}\n{value}\n{delim}\n")
}

fn outputs(plan: &Plan, path: &Path, runs: &[(String, bool)]) -> String {
    let invocations = serde_json::to_string(&plan.invocations).expect("invocations print");
    let checks: Vec<&str> = plan.checks.iter().map(|c| c.id.as_str()).collect();
    [
        output_block("all", if plan.all { "true" } else { "false" }),
        output_block("plan", &path.display().to_string()),
        output_block("plan_id", &plan.plan_id),
        output_block("invocations", &invocations),
        output_block("checks", &checks.join(" ")),
        output_block("tests", &plan.tests.len().to_string()),
        output_block("stage", plan.stage.map_or("", |s| s.name())),
        output_block(
            "reused",
            &plan
                .reused
                .as_ref()
                .map_or(String::new(), |r| r.pull_request.to_string()),
        ),
    ]
    .into_iter()
    .chain(
        runs.iter()
            .map(|(name, on)| output_block(name, if *on { "true" } else { "false" })),
    )
    .collect()
}

fn write_outputs(plan: &Plan, path: &Path, runs: &[(String, bool)]) -> Result<(), String> {
    let target = std::env::var_os("GITHUB_OUTPUT")
        .ok_or("--format github needs $GITHUB_OUTPUT, which GitHub Actions sets")?;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&target)
        .map_err(|e| format!("could not open $GITHUB_OUTPUT: {e}"))?;
    file.write_all(outputs(plan, path, runs).as_bytes())
        .map_err(|e| format!("could not write $GITHUB_OUTPUT: {e}"))
}

pub fn read_plan(cwd: &Path, plan_path: &Path) -> Result<Plan, String> {
    let text = std::fs::read_to_string(cwd.join(plan_path))
        .map_err(|e| format!("could not read {}: {e}", plan_path.display()))?;
    let version = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("{} isn't JSON: {e}", plan_path.display()))?
        .get("version")
        .and_then(serde_json::Value::as_u64);
    if version != Some(u64::from(VERSION)) {
        let found = version.map_or("no version".to_string(), |v| format!("plan version {v}"));
        return Err(format!(
            "{} is {found}; this fairlead reads version {VERSION}",
            plan_path.display()
        ));
    }
    serde_json::from_str(&text)
        .map_err(|e| format!("{} isn't a valid plan: {e}", plan_path.display()))
}

fn execute(
    cwd: &Path,
    plan_path: &Path,
    fail_fast: bool,
    results: Option<&Path>,
    judge: Option<&Path>,
    pick: &Pick,
) -> Result<ExitCode, String> {
    let mut plan = read_plan(cwd, plan_path)?;
    pick.check(cwd, &plan)?;
    let planned = plan.invocations.len();
    plan.invocations.retain(|i| pick.wants(&i.id));
    let ran_all = plan.invocations.len() == planned;
    let root = crate::graph_cmd::repo_root(cwd);
    let merge = judge.map(|p| read_plan(cwd, p)).transpose()?;
    let config = match &merge {
        Some(_) => Some(
            fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
                .map_err(|e| e.to_string())?
                .config,
        ),
        None => None,
    };
    let judge = match (&merge, &config) {
        (Some(m), Some(c)) => Some(crate::ci_judge::Judge::new(&root, c, m)),
        _ => None,
    };
    let started = std::time::Instant::now();
    let mut ran: Vec<crate::ci_report::Ran> = Vec::new();
    let mut judged: Vec<crate::ci_judge::Verdict> = Vec::new();
    let mut failed = Vec::new();
    let mut held = 0;
    for inv in &plan.invocations {
        let at = std::time::Instant::now();
        println!("fairlead: ({}) {}", inv.cwd, inv.argv.join(" "));
        let entry = plan.quarantine_of(inv);
        let (outcome, log) = crate::step::run("fairlead", &root, inv, entry, judge.is_some());
        ran.push(crate::ci_report::Ran::of(inv, outcome, at));
        match outcome {
            Outcome::Passed => {}
            Outcome::Held => held += 1,
            Outcome::Failed => {
                if let Some(j) = &judge {
                    match j.judge(inv, &log) {
                        Ok(v) => judged.extend(v),
                        Err(e) => eprintln!("fairlead: {e}"),
                    }
                }
                failed.push(inv.id.clone());
                if fail_fast {
                    break;
                }
            }
        }
    }
    crate::ci_judge::print(&judged);
    if let Some(path) = results {
        crate::ci_report::write_results(&cwd.join(path), &plan, ran, &judged, started)?;
    }
    if failed.is_empty() && held == 0 {
        let config = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()));
        if let Some(line) = config
            .ok()
            .and_then(|l| crate::reuse::record(&plan, &l.config, ran_all))
        {
            println!("{line}");
        }
    }
    if failed.is_empty() {
        let note = if held > 0 {
            format!(" ({held} failed as quarantined, not provable here)")
        } else {
            String::new()
        };
        println!(
            "fairlead: {} invocations passed{note}",
            plan.invocations.len()
        );
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("fairlead: failed: {}", failed.join(", "));
        Ok(ExitCode::FAILURE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_block_never_contains_its_own_delimiter() {
        let block = output_block("x", "a\nFAIRLEAD_EOF_0\nb");
        assert!(block.starts_with("x<<FAIRLEAD_EOF_1\n"));
        assert!(block.ends_with("\nFAIRLEAD_EOF_1\n"));
    }
}
