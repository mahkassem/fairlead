//! The plan, version 1: a public contract with its own JSON Schema. New
//! fields are additive, so readers accept fields they don't know and optional
//! fields are left out when empty; anything else bumps `version`. It lives in
//! core so replay and receipts can read plans without the planner.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::TestClass;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct Change {
    pub path: String,
    pub status: Status,
    /// For a rename, the old path; it counts as changed too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    pub version: u32,
    /// Stable for the same config, tree, base and changes.
    pub plan_id: String,
    pub fairlead_version: String,
    /// `sha256:` of the merged config.
    pub config_digest: String,
    /// The git tree id of HEAD when the working tree is clean, else
    /// `worktree:` and a hash of every file's path and blob id.
    pub tree_hash: String,
    /// The commit changes were measured from, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// HEAD's commit when the working tree matches it, else `worktree`.
    pub head: String,
    /// Everything is selected.
    pub all: bool,
    pub changed: Vec<Change>,
    /// Changed paths `plan.ignore` matched that nothing references.
    pub ignored: Vec<String>,
    pub tests: Vec<TestSelection>,
    pub checks: Vec<CheckSelection>,
    /// What to run, in order: each an argv in a working directory.
    pub invocations: Vec<Invocation>,
    /// Changed files nothing reaches, and what the plan did about each.
    pub unreached: Vec<UnreachedFile>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TestSelection {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    pub class: TestClass,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CheckSelection {
    pub id: String,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum InvocationKind {
    Runner,
    Check,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Invocation {
    /// The runner or check id.
    pub id: String,
    pub kind: InvocationKind,
    /// Repo-relative working directory.
    pub cwd: String,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnreachedFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// What was selected for it: `module`, `all`, or `none` under `warn`.
    pub selected: String,
}

/// The first reason a test or check was selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Reason {
    /// A changed path matched `plan.run_all`.
    RunAll { path: String },
    /// The test file itself changed.
    Changed,
    /// The test depends on a changed file: the chain runs from that file
    /// to the test, one edge per step.
    Import { chain: Vec<String> },
    /// An owner rule's `covers` matched a changed path.
    Owner {
        rule: usize,
        covers: String,
        changed: String,
    },
    /// Canary tests run in every plan.
    Canary,
    /// A changed file nothing reaches widened to this test.
    Unreached { path: String, policy: String },
    /// A check's `paths` matched this many changed files.
    Paths { matched: usize },
    /// A check watches a module the change affects.
    Modules { modules: Vec<String> },
    /// A check with no `paths` or `modules` runs in every plan.
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Warning {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
}

/// The JSON Schema for plan version 1, keys sorted so it prints the same
/// whichever `serde_json` features the build unified.
pub fn json_schema() -> serde_json::Value {
    sorted(serde_json::to_value(schemars::schema_for!(Plan)).expect("schema serializes"))
}

fn sorted(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let ordered: std::collections::BTreeMap<String, serde_json::Value> =
                map.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            serde_json::Value::Object(ordered.into_iter().collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sorted).collect())
        }
        other => other,
    }
}
