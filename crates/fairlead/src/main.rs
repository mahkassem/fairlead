//! The `fairlead` command. Each command arrives with its milestone; K1.1
//! adds `config`, K1.2 `graph`, K1.3 `plan`, `test --explain` and `ci`,
//! K2.1 `guard`.

mod agents_cmd;
mod bench_cmd;
mod blueprint;
mod brief_cmd;
mod ci_cmd;
mod ci_judge;
mod ci_report;
mod ci_workflow;
mod context_cmd;
mod coverage_cmd;
mod done_cmd;
mod escapes;
mod find_cmd;
mod graph_cmd;
mod guard_cmd;
mod hook_cmd;
mod hooks_cmd;
mod import_cmd;
mod import_rules_cmd;
mod init_cmd;
mod knowledge;
mod lessons_cmd;
mod mcp_cmd;
mod mcp_tools;
mod migrate_cmd;
mod migrate_notes;
mod plan_cmd;
mod receipt_cmd;
mod replay_cmd;
mod resume_cmd;
mod reuse;
mod score_cmd;
mod since_green;
mod skills_cmd;
mod skills_eval_cmd;
mod skills_report_cmd;
mod stage;
mod step;
mod tracker;
mod workspace_cmd;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use fairlead_core::config::{self, ConfigError, LoadOptions, Loaded};
use serde_json::Value;

#[derive(Parser)]
#[command(name = "fairlead", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Layer `fairlead.<NAME>.toml` and `fairlead.<NAME>.local.toml` over
    /// the project's config, as `FAIRLEAD_ENV` does.
    #[arg(
        long = "env",
        value_name = "NAME",
        global = true,
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    environment: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Report the binary, the platform and the config Fairlead would use here.
    Doctor {
        #[command(flatten)]
        args: score_cmd::DoctorArgs,
    },
    /// Write a first fairlead.toml from what the repository shows: its test runners, and a check for each language the graph doesn't read.
    Init {
        #[command(flatten)]
        args: init_cmd::InitArgs,
    },
    /// Validate, show or describe the config.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
        /// Override a value for this run, such as `tests.unreached=all`.
        #[arg(long = "set", value_name = "KEY=VALUE", global = true)]
        sets: Vec<String>,
    },
    /// Which tests and checks the changes since a base commit can affect.
    Plan {
        #[command(flatten)]
        changes: plan_cmd::Changes,
        /// Print the plan as JSON.
        #[arg(long)]
        json: bool,
        /// Also write the plan JSON to this file.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Print the plan's JSON Schema and exit.
        #[arg(long)]
        schema: bool,
    },
    /// Explain test selection.
    Test {
        #[command(flatten)]
        changes: plan_cmd::Changes,
        /// Why this test file or check is in the plan, or why it isn't.
        #[arg(long, value_name = "FILE_OR_CHECK")]
        explain: String,
    },
    /// Serve the plan, the brief and the other read-only commands to an agent over MCP (stdio).
    Mcp,
    /// Replay recorded CI failures against the planner.
    Replay {
        #[command(subcommand)]
        action: replay_cmd::ReplayAction,
    },
    /// Plan and run tests in CI.
    Ci {
        #[command(subcommand)]
        action: ci_cmd::CiAction,
    },
    /// Check the project's rules.
    Guard {
        #[command(subcommand)]
        action: guard_cmd::GuardAction,
        /// Override a config value for this run.
        #[arg(long = "set", value_name = "KEY=VALUE", global = true)]
        sets: Vec<String>,
    },
    /// Run the gate a change passes before it's finished: its planned tests
    /// and checks, `done.always` and the guard, recorded against the tree.
    Done {
        #[command(flatten)]
        args: done_cmd::DoneArgs,
    },
    /// Before an edit: what the paths reach, the tests and checks that will
    /// run, the rules that read them and the done gate, each with its source.
    Brief {
        #[command(flatten)]
        args: brief_cmd::BriefArgs,
    },
    /// The brief, then what to read before the edit: the lessons' bodies,
    /// the skills to load, the nearest README and the commits that last
    /// touched each file.
    Context {
        #[command(flatten)]
        args: context_cmd::ContextArgs,
    },
    /// For a new session: the last brief on this branch, what changed since,
    /// the done gate, `next` and the lessons the branch added.
    Resume {
        #[command(flatten)]
        args: resume_cmd::ResumeArgs,
    },
    /// After a change: what changed against the session's brief, the tests
    /// files outside it add, and the done gate for the tree as it is.
    Receipt {
        #[command(flatten)]
        args: receipt_cmd::ReceiptArgs,
    },
    /// The one step the change loop is waiting for: a brief, the done gate,
    /// a fix, the receipt, or nothing.
    Next {
        #[command(flatten)]
        args: receipt_cmd::NextArgs,
    },
    /// Write one lesson: a scope, evidence, and who or what taught it.
    Learn {
        #[command(flatten)]
        args: lessons_cmd::LearnArgs,
    },
    /// List the lessons, the ones due for review, or check them all.
    Lessons {
        #[command(subcommand)]
        action: lessons_cmd::LessonsAction,
    },
    /// Bring in what a team already keeps, such as a lessons document.
    Import {
        #[command(subcommand)]
        action: import_cmd::ImportAction,
    },
    /// Search the lessons, skills, docs headings and declared names, nearest
    /// the session's brief first; `--symbol` finds where a name is declared.
    Find {
        #[command(flatten)]
        args: find_cmd::FindArgs,
    },
    /// Keep a marked block in AGENTS.md and CLAUDE.md: the change loop's
    /// commands, the always-on lessons and the skill index.
    Agents {
        #[command(subcommand)]
        action: agents_cmd::AgentsAction,
    },
    /// Bring the hooks, the config's version floor and the version pins to
    /// this release, and list what changed since that needs a person.
    Migrate {
        #[command(flatten)]
        args: migrate_cmd::MigrateArgs,
    },
    /// Write each routed skill in the format each agent loads, or check that they're in sync.
    Skills {
        #[command(subcommand)]
        action: skills_cmd::SkillsAction,
    },
    /// Install, check or remove the Claude Code and git hooks that run the guard, the brief nudge, the Stop hook and `resume` at a session's start.
    Hooks {
        #[command(subcommand)]
        action: hooks_cmd::HooksAction,
    },
    /// Import a coverage run into the map `graph.coverage` reads.
    Coverage {
        #[command(subcommand)]
        action: coverage_cmd::CoverageAction,
    },
    /// Inspect the import graph.
    Graph {
        #[command(subcommand)]
        action: graph_cmd::GraphAction,
        /// Override a config value for this run.
        #[arg(long = "set", value_name = "KEY=VALUE", global = true)]
        sets: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Validate every layer and the merged result.
    Check,
    /// Print the merged config.
    Show {
        /// Print each value with the layer that set it.
        #[arg(long)]
        origin: bool,
    },
    /// Print the JSON Schema, for editor autocomplete.
    Schema,
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Where `fairlead` is on the PATH, if it is: the hooks run it by name.
fn on_path() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["fairlead.exe", "fairlead"]
    } else {
        &["fairlead"]
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

/// Hooks, the binary on the PATH, and what the event log recorded.
fn doctor_hooks(dir: &Path, loaded: Option<&Loaded>) -> String {
    let root = graph_cmd::repo_root(dir);
    let Some(git_dir) = fairlead_guard::git::git_dir(&root) else {
        return "hooks: not a git repository\n".to_string();
    };
    let config = loaded.map(|l| l.config.clone()).unwrap_or_default();
    let mut out: String = hooks_cmd::describe(&root, &config)
        .into_iter()
        .map(|l| l + "\n")
        .collect();
    out.push_str(&match (on_path(), hooks_cmd::package_runner(&root)) {
        (_, Some(runner)) => {
            format!("binary: the project's own copy, through `{runner}` where it isn't unpacked\n")
        }
        (Some(path), None) => format!("on PATH: {}\n", path.display()),
        (None, None) => {
            "on PATH: no; the hooks do nothing until `fairlead` is installed\n".to_string()
        }
    });
    let log =
        std::fs::read_to_string(git_dir.join("fairlead").join("events.jsonl")).unwrap_or_default();
    out.push_str(&fairlead_guard::summary::render(
        &fairlead_guard::summary::summarize(&log),
    ));
    out
}

fn doctor_report(dir: &Path, opts: &LoadOptions) -> String {
    let loaded = config::load(dir, opts);
    let mut hooks = doctor_hooks(dir, loaded.as_ref().ok());
    let mut agents = String::new();
    if let Ok(l) = &loaded {
        let root = graph_cmd::repo_root(dir);
        if let Some(line) = lessons_cmd::doctor_line(&root, &l.config.memory) {
            hooks.push_str(&line);
        }
        let root = if l.files.is_empty() {
            graph_cmd::repo_root(dir)
        } else {
            l.root.clone()
        };
        if let Some(line) = skills_cmd::doctor_line(&root, &l.config) {
            hooks.push_str(&line);
        }
        agents = agents_cmd::doctor_lines(&root, &l.config);
    }
    let config = match loaded {
        Ok(loaded) if loaded.files.is_empty() => "none found; defaults apply".to_string(),
        Ok(loaded) if loaded.problems.is_empty() => {
            format!("{} (valid)", loaded.files[0].display())
        }
        Ok(loaded) => format!(
            "{} ({} problems; run `fairlead config check`)",
            loaded.files[0].display(),
            loaded.problems.len()
        ),
        Err(e) => format!("invalid: {e}"),
    };
    let here = fairlead_tests::quarantine::Here::detect(&graph_cmd::repo_root(dir));
    let conditions: Vec<&str> = here.conditions.iter().map(|c| c.name()).collect();
    format!(
        "fairlead {}\nplatform: {}-{}\nconditions: {}\nconfig: {}\n{agents}{hooks}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if conditions.is_empty() {
            "none".to_string()
        } else {
            conditions.join(", ")
        },
        config
    )
}

/// Test files that don't map to exactly one runner, which only the tree can say.
fn check_tree(loaded: &Loaded) -> fairlead_lang::tree::Tree {
    let root = if loaded.files.is_empty() {
        graph_cmd::repo_root(&cwd())
    } else {
        loaded.root.clone()
    };
    fairlead_lang::tree::Tree::scan(&root)
}

fn runner_problems(loaded: &Loaded) -> Vec<String> {
    let tree = check_tree(loaded);
    fairlead_tests::testfiles::runner_problems(&tree, &loaded.config).unwrap_or_else(|e| vec![e])
}

fn owner_warnings(loaded: &Loaded) -> Vec<String> {
    if loaded.config.tests.owners.is_empty() || !loaded.problems.is_empty() {
        return Vec::new();
    }
    fairlead_tests::testfiles::idle_owners(&check_tree(loaded), &loaded.config).unwrap_or_default()
}

/// The conditions detected here and whether each `[[quarantine]]` entry
/// holds, so a person can see why one applies.
fn quarantine_lines(loaded: &Loaded) -> Vec<String> {
    use fairlead_tests::quarantine::{status, Here, Status};
    let entries = loaded.config.quarantine.items();
    if entries.is_empty() {
        return Vec::new();
    }
    let root = if loaded.files.is_empty() {
        graph_cmd::repo_root(&cwd())
    } else {
        loaded.root.clone()
    };
    let here = Here::detect(&root);
    let mut out = vec![format!("here: {}", here.describe())];
    for entry in entries {
        let state = match status(entry, &here) {
            Status::Holds(why) => format!("holds here ({}) until {}", why.join(", "), entry.until),
            Status::Expired => format!("ended on {}, so a failure here counts", entry.until),
            Status::Elsewhere(why) => format!("doesn't hold here: {why}"),
        };
        out.push(format!("quarantine {}: {state}", entry.target()));
    }
    out
}

fn check(loaded: &Loaded) -> ExitCode {
    for w in &loaded.warnings {
        eprintln!("warning: {}: {}", w.key, w.message);
    }
    for line in owner_warnings(loaded) {
        eprintln!("warning: {line}");
    }
    let mut problems = loaded.problems.clone();
    problems.extend(fairlead_core::config::plan_globs(&loaded.config));
    let runners = if problems.is_empty() {
        runner_problems(loaded)
    } else {
        Vec::new()
    };
    if !runners.is_empty() {
        for line in runners.iter().take(50) {
            eprintln!("tests.runners: {line}");
        }
        eprintln!(
            "{} test file(s) don't map to exactly one runner",
            runners.len()
        );
        return ExitCode::FAILURE;
    }
    if problems.is_empty() {
        let files: Vec<String> = loaded
            .files
            .iter()
            .map(|f| f.display().to_string())
            .collect();
        let source = if files.is_empty() {
            "defaults only".to_string()
        } else {
            files.join(" + ")
        };
        println!("config ok: {source}");
        for line in quarantine_lines(loaded) {
            println!("{line}");
        }
        return ExitCode::SUCCESS;
    }
    for p in &problems {
        eprintln!("{}: {}", p.key, p.message);
    }
    eprintln!("config has {} problem(s)", problems.len());
    ExitCode::FAILURE
}

/// Every leaf as `key = value  # layer`, lists printed whole.
fn with_origins(value: &Value, path: &str, loaded: &Loaded, out: &mut Vec<String>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                with_origins(child, &child_path, loaded, out);
            }
        }
        Value::Null => {}
        leaf => {
            let origin = loaded
                .origins
                .get(path)
                .map(String::as_str)
                .unwrap_or("default");
            out.push(format!("{path} = {leaf}  # {origin}"));
        }
    }
}

fn show(loaded: &Loaded, origin: bool) -> ExitCode {
    if origin {
        let mut lines = Vec::new();
        with_origins(&loaded.value, "", loaded, &mut lines);
        println!("{}", lines.join("\n"));
    } else {
        match toml::to_string_pretty(&loaded.config) {
            Ok(text) => print!("{text}"),
            Err(e) => {
                eprintln!("could not print the config: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn run_config(action: ConfigAction, sets: Vec<String>) -> ExitCode {
    if let ConfigAction::Schema = action {
        println!(
            "{}",
            serde_json::to_string_pretty(&config::json_schema()).expect("schema prints")
        );
        return ExitCode::SUCCESS;
    }
    let loaded = match config::load(&cwd(), &LoadOptions::from_process(sets)) {
        Ok(loaded) => loaded,
        Err(e) => return fail(&e),
    };
    match action {
        ConfigAction::Check => check(&loaded),
        ConfigAction::Show { origin } => show(&loaded, origin),
        ConfigAction::Schema => unreachable!("handled above"),
    }
}

fn fail(e: &ConfigError) -> ExitCode {
    eprintln!("{e}");
    ExitCode::FAILURE
}

/// A reader that stops early, such as `head`, closes the pipe under a
/// print, and with SIGPIPE ignored the print panics. End the way a process
/// killed by SIGPIPE does instead: quietly, with 141, never 0, so a gate cut
/// short isn't read as a pass. Every other panic keeps the default report.
#[cfg(unix)]
fn end_quietly_on_a_closed_pipe() {
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("");
        if message.starts_with("failed printing to std") && message.ends_with("(os error 32)") {
            std::process::exit(141);
        }
        report(info);
    }));
}

fn main() -> ExitCode {
    #[cfg(unix)]
    end_quietly_on_a_closed_pipe();
    let cli = Cli::parse();
    if let Some(name) = &cli.environment {
        // Set before any thread starts, so every config load sees it.
        std::env::set_var(config::ENV_NAME, name);
    }
    match cli.command {
        Some(Command::Doctor { args }) if args.score => {
            score_cmd::run(&args, &cwd(), &LoadOptions::from_process(Vec::new()))
        }
        Some(Command::Doctor { .. }) => {
            print!(
                "{}",
                doctor_report(&cwd(), &LoadOptions::from_process(Vec::new()))
            );
            ExitCode::SUCCESS
        }
        Some(Command::Init { args }) => init_cmd::run(args, &cwd()),
        Some(Command::Config { action, sets }) => run_config(action, sets),
        Some(Command::Graph { action, sets }) => graph_cmd::run(action, sets, &cwd()),
        Some(Command::Coverage { action }) => coverage_cmd::run(action, &cwd()),
        Some(Command::Guard { action, sets }) => guard_cmd::run(action, sets, &cwd()),
        Some(Command::Learn { args }) => lessons_cmd::learn(args, &cwd()),
        Some(Command::Lessons { action }) => lessons_cmd::run(action, &cwd()),
        Some(Command::Import { action }) => import_cmd::run(action, &cwd()),
        Some(Command::Skills { action }) => skills_cmd::run(action, &cwd()),
        Some(Command::Find { args }) => find_cmd::run(args, &cwd()),
        Some(Command::Agents { action }) => agents_cmd::run(action, &cwd()),
        Some(Command::Hooks { action }) => hooks_cmd::run(action, &cwd()),
        Some(Command::Migrate { args }) => migrate_cmd::run(args, &cwd()),
        Some(Command::Done { args }) => done_cmd::run(args, &cwd()),
        Some(Command::Brief { args }) => brief_cmd::run(args, &cwd()),
        Some(Command::Context { args }) => context_cmd::run(args, &cwd()),
        Some(Command::Resume { args }) => resume_cmd::run(args, &cwd()),
        Some(Command::Receipt { args }) => receipt_cmd::run(args, &cwd()),
        Some(Command::Next { args }) => receipt_cmd::run_next(args, &cwd()),
        Some(Command::Plan {
            changes,
            json,
            out,
            schema,
        }) => plan_cmd::run_plan(&cwd(), changes, json, out, schema),
        Some(Command::Ci { action }) => ci_cmd::run(action, &cwd()),
        Some(Command::Replay { action }) => replay_cmd::run(action),
        Some(Command::Mcp) => mcp_cmd::run(&cwd()),
        Some(Command::Test { changes, explain }) => {
            plan_cmd::run_explain(&cwd(), changes, &explain)
        }
        None => {
            println!(
                "fairlead {}: see `fairlead --help`",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fairlead-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        dir
    }

    #[test]
    fn reports_the_version_and_that_defaults_apply_without_a_config() {
        let root = scratch("missing");
        let report = doctor_report(&root.join("a/b"), &LoadOptions::default());
        assert!(report.starts_with(&format!("fairlead {}\n", env!("CARGO_PKG_VERSION"))));
        assert!(report.contains("config: none found; defaults apply"));
    }

    #[test]
    fn reports_an_invalid_config() {
        let root = scratch("invalid");
        fs::write(root.join("fairlead.toml"), "[tests]\nunreachd = \"all\"\n").unwrap();
        let report = doctor_report(&root.join("a/b"), &LoadOptions::default());
        assert!(
            report.contains("config: invalid: fairlead.toml: tests.unreachd"),
            "{report}"
        );
    }
}
