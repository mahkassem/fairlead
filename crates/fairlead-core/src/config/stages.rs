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
    /// When `ci workflow` schedules the full stage, as a cron line in UTC,
    /// once any whole suite waits for it.
    pub nightly: String,
}

impl Default for Stages {
    fn default() -> Self {
        Stages {
            environments: strings(&["main"]),
            full_label: "run-everything".into(),
            reuse: true,
            nightly: "17 19 * * 1-5".into(),
        }
    }
}

/// Where a project is in its life. `early` and `testing` have no live
/// deployment, so a change proves what it reaches and whole suites run at
/// the end of the day; `live` keeps every step as it's configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Early,
    Testing,
    Live,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::Early => "early",
            Phase::Testing => "testing",
            Phase::Live => "live",
        }
    }

    /// Whether anything is deployed for people to use.
    pub fn is_live(self) -> bool {
        self == Phase::Live
    }
}

impl Config {
    /// Whether `ci plan` reads a stage when none is asked for: `[stages]`
    /// is set, or a runner or check says when it, or its whole suite, runs.
    pub fn uses_stages(&self) -> bool {
        self.stages.is_some()
            || self.tests.runners.items().iter().any(|r| r.from.is_some())
            || self.checks.items().iter().any(|c| c.from.is_some())
            || self.has_all_from()
    }

    /// Whether any runner or check runs its whole suite only from a later
    /// stage than its own.
    pub fn has_all_from(&self) -> bool {
        self.tests
            .runners
            .items()
            .iter()
            .any(|r| self.all_from_of(r.all_from).is_some())
            || self
                .checks
                .items()
                .iter()
                .any(|c| self.all_from_of(c.all_from).is_some())
    }

    /// A step's `all_from`, or the phase's default when it sets none: the
    /// full stage before a live deployment, none once there is one.
    pub fn all_from_of(&self, set: Option<CiStage>) -> Option<CiStage> {
        set.or(match self.phase {
            Some(Phase::Early | Phase::Testing) => Some(CiStage::Full),
            Some(Phase::Live) | None => None,
        })
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
    if stages.nightly.split_whitespace().count() != 5 {
        problems.push(super::validate::problem(
            "stages.nightly",
            "a cron line has five fields, such as \"17 19 * * 1-5\"",
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
