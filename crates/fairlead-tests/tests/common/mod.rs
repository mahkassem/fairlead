//! A synthetic repository, a config, a list of changes, and the plan.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use fairlead_core::config::Config;
use fairlead_core::plan::{Change, Plan, Reason, Status};
use fairlead_lang::build;
use fairlead_tests::{plan, Input};

pub fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-plan-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).unwrap();
    for (path, text) in files {
        let full = dir.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }
    dir
}

pub fn config(toml_text: &str) -> Config {
    let mut config: Config = toml::from_str(toml_text).unwrap();
    config.graph.cache = false;
    config
}

pub fn modified(path: &str) -> Change {
    Change {
        path: path.into(),
        status: Status::Modified,
        from: None,
    }
}

pub fn deleted(path: &str) -> Change {
    Change {
        path: path.into(),
        status: Status::Deleted,
        from: None,
    }
}

pub fn renamed(from: &str, to: &str) -> Change {
    Change {
        path: to.into(),
        status: Status::Renamed,
        from: Some(from.into()),
    }
}

pub fn try_plan(dir: &Path, config: &Config, changes: Vec<Change>) -> Result<Plan, String> {
    let mut scan = build(dir, config).unwrap();
    plan(
        &mut scan,
        config,
        Input {
            changes,
            base: Some("base".into()),
            head: "worktree".into(),
            config_digest: "sha256:test".into(),
            tree_hash: "worktree:test".into(),
        },
    )
}

pub fn run(dir: &Path, config: &Config, changes: Vec<Change>) -> Plan {
    try_plan(dir, config, changes).unwrap()
}

/// Selected test paths, sorted.
pub fn tests(plan: &Plan) -> Vec<&str> {
    let mut paths: Vec<&str> = plan.tests.iter().map(|t| t.path.as_str()).collect();
    paths.sort();
    paths
}

pub fn reason<'a>(plan: &'a Plan, path: &str) -> &'a Reason {
    &plan
        .tests
        .iter()
        .find(|t| t.path == path)
        .unwrap_or_else(|| panic!("{path} not selected; selected: {:?}", tests(plan)))
        .reason
}

pub const VITEST: &str = r#"
[[tests.runners]]
id = "vitest"
match = ["**"]
command = ["vitest", "run", "{files}"]
"#;
