//! The `fairlead` command. Each command arrives with its milestone; K1.1
//! adds `config`.

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
}

#[derive(Subcommand)]
enum Command {
    /// Report the binary, the platform and the config Fairlead would use here.
    Doctor,
    /// Validate, show or describe the config.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
        /// Override a value for this run, such as `tests.unreached=all`.
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

fn doctor_report(dir: &Path, opts: &LoadOptions) -> String {
    let config = match config::load(dir, opts) {
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
    format!(
        "fairlead {}\nplatform: {}-{}\nconfig: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        config
    )
}

fn check(loaded: &Loaded) -> ExitCode {
    if loaded.problems.is_empty() {
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
        return ExitCode::SUCCESS;
    }
    for p in &loaded.problems {
        eprintln!("{}: {}", p.key, p.message);
    }
    eprintln!("config has {} problem(s)", loaded.problems.len());
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

fn main() -> ExitCode {
    match Cli::parse().command {
        Some(Command::Doctor) => {
            print!(
                "{}",
                doctor_report(&cwd(), &LoadOptions::from_process(Vec::new()))
            );
            ExitCode::SUCCESS
        }
        Some(Command::Config { action, sets }) => run_config(action, sets),
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
