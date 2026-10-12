//! `[tracker]`: where a change's task lives, so the brief can show it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Config, Problem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TrackerKind {
    /// No tracker.
    #[default]
    None,
    /// A command of the team's own: one argv per operation, JSON out.
    Command,
    /// GitHub issues of the repository's `origin`.
    Github,
    /// The agent's own tracker tool: Fairlead names the task and calls nothing.
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Tracker {
    pub kind: TrackerKind,
    /// A regex every task id must match whole, before it goes anywhere.
    /// Defaults to `[0-9]+` for `github`; `command` and `agent` need one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A regex that finds the task id in the branch name: its first capture
    /// group, else the whole match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// For `command`: the argv that prints one task as JSON; the id is
    /// appended as its last argument.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub get: Vec<String>,
    /// How long one tracker call may take, in seconds.
    pub timeout: u64,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker {
            kind: TrackerKind::None,
            id: None,
            branch: None,
            get: Vec::new(),
            timeout: 20,
        }
    }
}

impl Tracker {
    pub fn is_unset(&self) -> bool {
        *self == Tracker::default()
    }

    /// The id pattern in force: the configured one, or GitHub's numbers.
    pub fn id_pattern(&self) -> Option<&str> {
        match (&self.id, self.kind) {
            (Some(id), _) => Some(id),
            (None, TrackerKind::Github) => Some("[0-9]+"),
            _ => None,
        }
    }
}

pub(super) fn validate(config: &Config, problems: &mut Vec<Problem>) {
    let t = &config.tracker;
    if t.kind == TrackerKind::None {
        return;
    }
    let p = |key: &str, message: &str| super::validate::problem(format!("tracker.{key}"), message);
    if t.id_pattern().is_none() {
        problems.push(p("id", "needs a regex for task ids, such as `T[0-9]+`"));
    }
    for (key, pattern) in [("id", &t.id), ("branch", &t.branch)] {
        if let Some(pattern) = pattern {
            if let Err(e) = regex::Regex::new(pattern) {
                problems.push(p(key, &format!("isn't a regex: {e}")));
            }
        }
    }
    if t.branch.is_none() {
        problems.push(p(
            "branch",
            "needs a regex that finds the task id in a branch name",
        ));
    }
    if t.kind == TrackerKind::Command && t.get.is_empty() {
        problems.push(p("get", "needs the command that prints a task as JSON"));
    }
    if t.timeout == 0 || t.timeout > 120 {
        problems.push(p("timeout", "is seconds, from 1 to 120"));
    }
}
