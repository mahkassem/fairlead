//! The `fairlead` command. K0 ships the version and a `doctor` that reports
//! what it can see; every other command arrives with its milestone.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

const CONFIG_NAMES: [&str; 3] = ["fairlead.toml", "fairlead.yaml", "fairlead.yml"];

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
}

/// The nearest config file, walking up from `start` to the filesystem root.
fn find_config(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        CONFIG_NAMES
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn doctor_report(cwd: &Path) -> String {
    let config = match find_config(cwd) {
        Some(path) => path.display().to_string(),
        None => "none found (run `fairlead init` once it exists)".to_string(),
    };
    format!(
        "fairlead {}\nplatform: {}-{}\nconfig: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        config
    )
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Doctor) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            print!("{}", doctor_report(&cwd));
            ExitCode::SUCCESS
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
        dir
    }

    #[test]
    fn finds_the_nearest_config_walking_up() {
        let root = scratch("walk");
        fs::write(root.join("fairlead.toml"), "").unwrap();
        assert_eq!(
            find_config(&root.join("a/b")),
            Some(root.join("fairlead.toml"))
        );
    }

    #[test]
    fn prefers_the_closest_directory_over_an_outer_one() {
        let root = scratch("closest");
        fs::write(root.join("fairlead.toml"), "").unwrap();
        fs::write(root.join("a/fairlead.yaml"), "").unwrap();
        assert_eq!(
            find_config(&root.join("a/b")),
            Some(root.join("a/fairlead.yaml"))
        );
    }

    #[test]
    fn reports_the_version_and_a_missing_config() {
        let root = scratch("missing");
        let report = doctor_report(&root.join("a/b"));
        assert!(report.starts_with(&format!("fairlead {}\n", env!("CARGO_PKG_VERSION"))));
        assert!(report.contains("config: none found"));
    }
}
