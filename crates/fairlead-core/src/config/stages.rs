//! CI stages: when in a change's life each runner and check runs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{strings, Config, List, Problem};

/// The stages of a change, earliest first. A step runs at its `from` stage
/// and every later one.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CiStage {
    /// A draft pull request.
    Draft,
    /// A pull request ready for review, or a push to a branch that isn't an environment.
    Ready,
    /// A push to an environment branch.
    Merge,
    /// A scheduled or manual run, or a pull request carrying the full label.
    Full,
}

impl CiStage {
    pub const ALL: [CiStage; 4] = [
        CiStage::Draft,
        CiStage::Ready,
        CiStage::Merge,
        CiStage::Full,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CiStage::Draft => "draft",
            CiStage::Ready => "ready",
            CiStage::Merge => "merge",
            CiStage::Full => "full",
        }
    }

    pub fn parse(text: &str) -> Option<CiStage> {
        CiStage::ALL.into_iter().find(|s| s.name() == text)
    }
}

impl std::fmt::Display for CiStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Stages {
    /// Branches whose pushes are the merge stage.
    pub environments: List<String>,
    /// A pull request label that runs everything.
    pub full_label: String,
    /// At merge, skip the ready stage's steps when the pull request passed
    /// them on the same tree.
    pub reuse: bool,
}

impl Default for Stages {
    fn default() -> Self {
        Stages {
            environments: strings(&["main"]),
            full_label: "run-everything".into(),
            reuse: true,
        }
    }
}

impl Config {
    /// Whether `ci plan` reads a stage when none is asked for: `[stages]`
    /// is set, or a runner or check says when it runs.
    pub fn uses_stages(&self) -> bool {
        self.stages.is_some()
            || self.tests.runners.items().iter().any(|r| r.from.is_some())
            || self.checks.items().iter().any(|c| c.from.is_some())
    }

    /// `[stages]` as set, or its defaults.
    pub fn stages_or_default(&self) -> Stages {
        self.stages.clone().unwrap_or_default()
    }
}

pub(super) fn validate(config: &Config, problems: &mut Vec<Problem>) {
    let Some(stages) = &config.stages else {
        return;
    };
    if stages
        .environments
        .items()
        .iter()
        .any(|b| b.trim().is_empty())
    {
        problems.push(super::validate::problem(
            "stages.environments",
            "a branch name can't be empty",
        ));
    }
    if stages.full_label.trim().is_empty() {
        problems.push(super::validate::problem(
            "stages.full_label",
            "can't be empty; a label nobody uses never matches",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_order_earliest_first_and_round_trip_by_name() {
        assert!(CiStage::Draft < CiStage::Ready && CiStage::Ready < CiStage::Merge);
        assert!(CiStage::Merge < CiStage::Full);
        for s in CiStage::ALL {
            assert_eq!(CiStage::parse(s.name()), Some(s));
        }
        assert_eq!(CiStage::parse("auto"), None);
    }
}
