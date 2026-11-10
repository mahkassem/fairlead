//! Setup mistakes a config can load with, said where a person or an agent
//! looks: `config check` and `doctor`. Each line names what is wrong and
//! the one thing that fixes it.

use std::path::Path;

use fairlead_core::config::{Config, Invoke};

/// What a runner's command uses to name the tests it runs.
const SELECTORS: [&str; 5] = ["{files}", "{packages}", "{class}", "{classes}", "{dir}"];

/// A runner whose command names no test: it runs its whole suite whenever
/// one of its tests is selected, however small the change.
pub fn runner_warnings(config: &Config) -> Vec<String> {
    config
        .tests
        .runners
        .items()
        .iter()
        .filter(|r| r.invoke == Invoke::Once)
        .filter(|r| {
            !r.command
                .iter()
                .any(|arg| SELECTORS.iter().any(|s| arg.contains(s)))
        })
        .map(|r| {
            format!(
                "tests.runners `{}`: its command names no `{{files}}`, so it runs its whole suite whenever any of its tests is selected; put `{{files}}` where the test files go",
                r.id
            )
        })
        .collect()
}

/// In a repository set up with only the guard: the runner it can have now,
/// as what was found and the edit that adds it.
pub fn appeared(root: &Path, config: &Config) -> Option<(String, &'static str)> {
    if config.profile.is_full() {
        return None;
    }
    let ids = crate::init_cmd::runners_now(root);
    if ids.is_empty() {
        return None;
    }
    Some((
        format!(
            "this repository has tests `{}` can run, and the guard-only profile plans none",
            ids.join(", ")
        ),
        "`fairlead init --dry-run` prints the runner; add it to fairlead.toml and remove `profile = \"guard\"`",
    ))
}

/// `appeared`, as `doctor`'s line.
pub fn tests_appeared(root: &Path, config: &Config) -> Option<String> {
    appeared(root, config).map(|(what, fix)| format!("tests: {what}. {fix}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Config {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn a_runner_that_names_no_test_is_warned_about_and_one_that_does_is_not() {
        let c = config(
            r#"
[[tests.runners]]
id = "whole"
match = ["e2e/**"]
command = ["npx", "playwright", "test"]

[[tests.runners]]
id = "named"
match = ["test/**"]
command = ["npx", "vitest", "run", "{files}"]

[[tests.runners]]
id = "go"
match = ["**/*_test.go"]
command = ["go", "test", "{packages}"]

[[tests.runners]]
id = "per"
match = ["packages/**"]
invoke = "per-module"
cwd = "{module}"
command = ["npm", "test"]
"#,
        );
        let warnings = runner_warnings(&c);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("`whole`"), "{warnings:?}");
    }
}
