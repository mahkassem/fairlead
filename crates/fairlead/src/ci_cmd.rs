//! `fairlead ci plan` and `fairlead ci run --plan`: the plan for a CI job,
//! as JSON and GitHub step outputs, and a runner that executes a plan's
//! invocations in order.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Subcommand, ValueEnum};
use fairlead_core::plan::{Plan, VERSION};

use crate::plan_cmd::{make, Changes};

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
    },
    /// Run a plan's invocations in order; exits non-zero if any fails.
    Run {
        #[arg(long, value_name = "PATH")]
        plan: PathBuf,
        /// Stop at the first failing invocation instead of running the rest.
        #[arg(long)]
        fail_fast: bool,
    },
}

pub fn run(action: CiAction, cwd: &Path) -> ExitCode {
    let result = match action {
        CiAction::Plan {
            changes,
            head,
            format,
            out,
        } => {
            let out = out.unwrap_or_else(default_out);
            plan(cwd, &changes, head.as_deref(), format, &out)
        }
        CiAction::Run { plan, fail_fast } => execute(cwd, &plan, fail_fast),
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

fn head_matches(cwd: &Path, expected: &str) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    let actual = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let same = !actual.is_empty()
        && !expected.is_empty()
        && (actual.starts_with(expected) || expected.starts_with(&actual));
    if same {
        return Ok(());
    }
    Err(format!(
        "HEAD is {actual}, not {expected}: check out the head commit before planning, since the plan reads the working tree"
    ))
}

fn plan(
    cwd: &Path,
    changes: &Changes,
    head: Option<&str>,
    format: Format,
    out: &Path,
) -> Result<ExitCode, String> {
    if let Some(head) = head {
        head_matches(cwd, head)?;
    }
    let planned = make(cwd, changes)?;
    let json = serde_json::to_string_pretty(&planned.plan).expect("plan prints");
    let path = cwd.join(out);
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    if format == Format::Github {
        write_outputs(&planned.plan, &path)?;
    }
    println!(
        "plan {}: {} tests, {} checks, {} invocations{} -> {}",
        planned.plan.plan_id,
        planned.plan.tests.len(),
        planned.plan.checks.len(),
        planned.plan.invocations.len(),
        if planned.plan.all {
            " (everything)"
        } else {
            ""
        },
        path.display()
    );
    Ok(ExitCode::SUCCESS)
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

fn outputs(plan: &Plan, path: &Path) -> String {
    let invocations = serde_json::to_string(&plan.invocations).expect("invocations print");
    let checks: Vec<&str> = plan.checks.iter().map(|c| c.id.as_str()).collect();
    [
        output_block("all", if plan.all { "true" } else { "false" }),
        output_block("plan", &path.display().to_string()),
        output_block("plan_id", &plan.plan_id),
        output_block("invocations", &invocations),
        output_block("checks", &checks.join(" ")),
        output_block("tests", &plan.tests.len().to_string()),
    ]
    .concat()
}

fn write_outputs(plan: &Plan, path: &Path) -> Result<(), String> {
    let target = std::env::var_os("GITHUB_OUTPUT")
        .ok_or("--format github needs $GITHUB_OUTPUT, which GitHub Actions sets")?;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&target)
        .map_err(|e| format!("could not open $GITHUB_OUTPUT: {e}"))?;
    file.write_all(outputs(plan, path).as_bytes())
        .map_err(|e| format!("could not write $GITHUB_OUTPUT: {e}"))
}

fn execute(cwd: &Path, plan_path: &Path, fail_fast: bool) -> Result<ExitCode, String> {
    let text = std::fs::read_to_string(cwd.join(plan_path))
        .map_err(|e| format!("could not read {}: {e}", plan_path.display()))?;
    let version = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("{} isn't JSON: {e}", plan_path.display()))?
        .get("version")
        .and_then(serde_json::Value::as_u64);
    if version != Some(u64::from(VERSION)) {
        return Err(format!(
            "{} is plan version {version:?}; this fairlead reads version {VERSION}",
            plan_path.display()
        ));
    }
    let plan: Plan = serde_json::from_str(&text)
        .map_err(|e| format!("{} isn't a valid plan: {e}", plan_path.display()))?;
    let root = crate::graph_cmd::repo_root(cwd);
    let mut failed = Vec::new();
    for inv in &plan.invocations {
        let Some((program, args)) = inv.argv.split_first() else {
            continue;
        };
        println!("fairlead: ({}) {}", inv.cwd, inv.argv.join(" "));
        let status = Command::new(program)
            .args(args)
            .current_dir(root.join(&inv.cwd))
            .status();
        let ok = matches!(&status, Ok(s) if s.success());
        if let Err(e) = &status {
            eprintln!("fairlead: couldn't start {program}: {e}");
        }
        if !ok {
            failed.push(inv.id.clone());
            if fail_fast {
                break;
            }
        }
    }
    if failed.is_empty() {
        println!("fairlead: {} invocations passed", plan.invocations.len());
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
