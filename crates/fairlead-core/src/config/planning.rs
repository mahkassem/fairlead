//! The `[plan]` section. A `plan.run_all` entry is a glob, which runs
//! everything whatever kind of change reaches a matching path, or
//! `{ paths, on }`, which runs everything only for the kinds of change `on`
//! names.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::validate::{problem, Problem};
use super::{strings, Config, List};
use crate::plan::Status;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Plan {
    /// A changed path matching any of these selects everything; an entry
    /// with `on` does so only for the kinds of change it names.
    pub run_all: List<RunAll>,
    /// A changed path matching these that reaches no test selects nothing,
    /// instead of falling to `tests.unreached`; one that reaches tests still
    /// selects them, and a changed test file always runs.
    pub ignore: List<String>,
    /// `all`: a changed lockfile selects everything. `scope`: a changed pnpm
    /// lockfile selects only the workspace packages whose resolved
    /// dependencies changed; opt-in while the benchmarks gather evidence.
    pub lockfile: LockfileMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LockfileMode {
    Scope,
    #[default]
    All,
}

impl Default for Plan {
    fn default() -> Self {
        Plan {
            run_all: strings(&[
                "package.json",
                "package-lock.json",
                "pnpm-lock.yaml",
                "pnpm-workspace.yaml",
                "yarn.lock",
                "bun.lock",
                "bun.lockb",
                "tsconfig*.json",
                "turbo.json",
                "nx.json",
                "**/project.json",
                "**/vitest.config.*",
                "**/vitest.workspace.*",
                "**/jest.config.*",
                "**/playwright.config.*",
                "go.work",
                "go.work.sum",
                "composer.json",
                "composer.lock",
                "pyproject.toml",
                "setup.py",
                "setup.cfg",
                "requirements*.txt",
                "poetry.lock",
                "uv.lock",
                "Pipfile.lock",
                ".github/workflows/**",
            ]),
            ignore: strings(&[
                "*.md",
                ".changeset/**",
                "docs/**",
                "**/README.md",
                "**/CHANGELOG.md",
                "LICENSE*",
            ]),
            lockfile: LockfileMode::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, expecting = "a glob, or { paths = [...], on = [...] }")]
pub enum RunAll {
    /// Any change to a matching path runs everything.
    Glob(String),
    /// Only a change of a kind `on` names runs everything; any other change
    /// to a matching path goes through the graph.
    Kinds(RunAllKinds),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunAllKinds {
    /// A glob, or a list of globs.
    pub paths: Globs,
    /// `added`, `modified`, `deleted` or `renamed`. A rename matches on its
    /// old path and its new one.
    #[serde(default)]
    pub on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Globs {
    One(String),
    Many(Vec<String>),
}

impl From<&str> for RunAll {
    fn from(glob: &str) -> Self {
        RunAll::Glob(glob.to_string())
    }
}

impl RunAll {
    pub fn globs(&self) -> &[String] {
        match self {
            RunAll::Glob(glob)
            | RunAll::Kinds(RunAllKinds {
                paths: Globs::One(glob),
                ..
            }) => std::slice::from_ref(glob),
            RunAll::Kinds(RunAllKinds {
                paths: Globs::Many(globs),
                ..
            }) => globs,
        }
    }

    /// The kinds of change this entry runs everything for: `None` for any.
    pub fn kinds(&self) -> Result<Option<Vec<Status>>, String> {
        let RunAll::Kinds(entry) = self else {
            return Ok(None);
        };
        if entry.on.is_empty() {
            return Err(EMPTY_ON.into());
        }
        let kinds = entry
            .on
            .iter()
            .map(|name| Status::from_name(name).ok_or_else(|| unknown_kind(name)))
            .collect::<Result<_, _>>()?;
        Ok(Some(kinds))
    }
}

const EMPTY_ON: &str = "names no kind of change, so it would never run everything; list some of `added`, `modified`, `deleted` and `renamed`, or write the glob alone for every kind";

fn unknown_kind(name: &str) -> String {
    format!("`{name}` isn't a kind of change; use `added`, `modified`, `deleted` or `renamed`")
}

/// `on` names known kinds and at least one, and `paths` at least one glob.
pub(super) fn validate(config: &Config, problems: &mut Vec<Problem>) {
    for (i, entry) in config.plan.run_all.items().iter().enumerate() {
        let RunAll::Kinds(kinds) = entry else {
            continue;
        };
        if entry.globs().is_empty() {
            problems.push(problem(
                format!("plan.run_all[{i}].paths"),
                "needs at least one glob",
            ));
        }
        if kinds.on.is_empty() {
            problems.push(problem(format!("plan.run_all[{i}].on"), EMPTY_ON));
        }
        for (j, name) in kinds.on.iter().enumerate() {
            if Status::from_name(name).is_none() {
                problems.push(problem(
                    format!("plan.run_all[{i}].on[{j}]"),
                    unknown_kind(name),
                ));
            }
        }
    }
}

/// Each entry's globs with the key `config check` names them by.
pub(super) fn keyed_globs(config: &Config) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    for (i, entry) in config.plan.run_all.items().iter().enumerate() {
        match entry {
            RunAll::Glob(glob) => out.push((format!("plan.run_all[{i}]"), glob.as_str())),
            RunAll::Kinds(RunAllKinds {
                paths: Globs::One(glob),
                ..
            }) => out.push((format!("plan.run_all[{i}].paths"), glob.as_str())),
            RunAll::Kinds(RunAllKinds {
                paths: Globs::Many(globs),
                ..
            }) => out.extend(
                globs
                    .iter()
                    .enumerate()
                    .map(|(j, g)| (format!("plan.run_all[{i}].paths[{j}]"), g.as_str())),
            ),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(toml_text: &str) -> Vec<RunAll> {
        let config: Config = toml::from_str(toml_text).unwrap();
        config.plan.run_all.items().to_vec()
    }

    #[test]
    fn a_glob_matches_every_kind_and_a_table_only_its_own() {
        let found = entries(
            "[plan]\nrun_all = [\"a/**\", { paths = \"m/**\", on = [\"modified\", \"renamed\"] }, { paths = [\"x\", \"y\"], on = [\"deleted\"] }]\n",
        );
        assert_eq!(found[0].globs(), ["a/**"]);
        assert_eq!(found[0].kinds(), Ok(None));
        assert_eq!(found[1].globs(), ["m/**"]);
        assert_eq!(
            found[1].kinds(),
            Ok(Some(vec![Status::Modified, Status::Renamed]))
        );
        assert_eq!(found[2].globs(), ["x", "y"]);
        assert_eq!(found[2].kinds(), Ok(Some(vec![Status::Deleted])));
    }

    #[test]
    fn an_unknown_or_missing_kind_is_an_error() {
        let found = entries(
            "[plan]\nrun_all = [{ paths = \"m/**\", on = [\"edited\"] }, { paths = \"m/**\" }]\n",
        );
        assert!(found[0].kinds().unwrap_err().contains("`edited`"));
        assert!(found[1].kinds().unwrap_err().contains("names no kind"));
    }

    #[test]
    fn keyed_globs_name_each_glob_where_it_is_written() {
        let config: Config = toml::from_str(
            "[plan]\nrun_all = { replace = [\"a\", { paths = \"b\", on = [\"added\"] }, { paths = [\"c\", \"d\"], on = [\"added\"] }] }\n",
        )
        .unwrap();
        let keys: Vec<(String, &str)> = keyed_globs(&config);
        let keys: Vec<(&str, &str)> = keys.iter().map(|(k, g)| (k.as_str(), *g)).collect();
        assert_eq!(
            keys,
            [
                ("plan.run_all[0]", "a"),
                ("plan.run_all[1].paths", "b"),
                ("plan.run_all[2].paths[0]", "c"),
                ("plan.run_all[2].paths[1]", "d"),
            ]
        );
    }
}
