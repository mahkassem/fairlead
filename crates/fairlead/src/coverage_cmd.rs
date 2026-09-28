//! `fairlead coverage import`: a coverage run's report, turned into the
//! coverage map the plan reads, stamped with the commit it tested.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Subcommand, ValueEnum};
use fairlead_core::config::{self, LoadOptions};
use fairlead_core::coverage::{today, CoverageMap, VERSION};
use fairlead_lang::coverage::{from_coverage_py, from_phpunit_xml};

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    /// PHPUnit's `--coverage-xml` directory.
    PhpunitXml,
    /// coverage.py's `coverage json --show-contexts`, run with `--cov-context=test`.
    CoveragePy,
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Format::PhpunitXml => "phpunit-xml",
            Format::CoveragePy => "coverage-py",
        }
    }
}

#[derive(Subcommand)]
pub enum CoverageAction {
    /// Turn a coverage run's report into the coverage map `graph.coverage` names.
    Import {
        #[arg(long, value_enum)]
        format: Format,
        /// The report: a directory for `phpunit-xml`, a JSON file for `coverage-py`.
        report: PathBuf,
        /// Where to write the map; `graph.coverage.map` by default.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
}

fn head(root: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn run(action: CoverageAction, cwd: &Path) -> ExitCode {
    let CoverageAction::Import {
        format,
        report,
        out,
    } = action;
    let loaded = match config::load(cwd, &LoadOptions::from_process(Vec::new())) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    let Some(out) = out.map(|o| cwd.join(o)).or_else(|| {
        loaded
            .config
            .graph
            .coverage
            .as_ref()
            .map(|c| root.join(&c.map))
    }) else {
        eprintln!("name the map with --out, or set graph.coverage.map");
        return ExitCode::FAILURE;
    };
    let scan = match fairlead_lang::build(&root, &loaded.config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not read {}: {e}", root.display());
            return ExitCode::FAILURE;
        }
    };
    let report = cwd.join(report);
    let converted = match format {
        Format::PhpunitXml => from_phpunit_xml(&report, &scan.tree, &scan.autoload),
        Format::CoveragePy => std::fs::read_to_string(&report)
            .map_err(|e| format!("couldn't read {}: {e}", report.display()))
            .and_then(|text| from_coverage_py(&text, &scan.tree)),
    };
    let (tests, unknown) = match converted {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let files: std::collections::BTreeSet<&String> = tests.values().flatten().collect();
    let map = CoverageMap {
        version: VERSION,
        commit: head(&root),
        created: today(),
        source: format.name().into(),
        tests: tests
            .iter()
            .map(|(t, f)| (t.clone(), f.iter().cloned().collect()))
            .collect(),
    };
    let text = serde_json::to_string_pretty(&map).expect("map serializes") + "\n";
    if let Err(e) = std::fs::write(&out, text) {
        eprintln!("couldn't write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    println!(
        "coverage map: {} tests ran {} files; wrote {}",
        map.tests.len(),
        files.len(),
        out.display()
    );
    if !unknown.is_empty() {
        let shown: Vec<&str> = unknown.iter().take(5).map(String::as_str).collect();
        println!(
            "  {} test names matched no file here, such as {}",
            unknown.len(),
            shown.join(", ")
        );
    }
    ExitCode::SUCCESS
}
