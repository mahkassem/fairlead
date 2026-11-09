//! The receipt's account of where the gate's steps ran: each step's CI
//! stage and platform, the planned steps no run on this tree reached, and
//! those this machine can't prove, with where they are proved instead.

use std::collections::BTreeMap;

use fairlead_core::config::CiStage;
use fairlead_core::plan::InvocationKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plan_cmd::Planned;

/// One step of the newest `done` run for the tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RanStep {
    pub id: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

/// A step this machine can't prove, why, and where it's proved instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unverifiable {
    pub step: String,
    /// Such as `needs macos` or `needs "postgres"`.
    pub why: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proved_in: Option<String>,
}

/// The steps a `done` event records.
pub fn ran(event: &Value) -> Vec<RanStep> {
    event["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| RanStep {
            id: s["id"].as_str().unwrap_or_default().to_string(),
            passed: s["passed"] == true,
            held: s["quarantined"] == true,
            stage: s["stage"].as_str().map(String::from),
            os: s["os"].as_str().map(String::from),
            arch: s["arch"].as_str().map(String::from),
        })
        .collect()
}

/// The steps the gate plans for this tree that no step of its newest run
/// covers, nor a need left out: a run stopped early, or none ran.
pub fn never_run(planned: &Planned, ran: &[RanStep], left_out: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in crate::done_cmd::steps(planned).unwrap_or_default() {
        let covered = ran.iter().any(|r| r.id == s.id) || left_out.contains(&s.id);
        if !covered && !out.contains(&s.id) {
            out.push(s.id);
        }
    }
    out
}

/// What this machine can't prove: the steps an unmet need left out, and
/// those a `[[quarantine]]` entry held here.
pub fn unverifiable(
    planned: &Planned,
    not_run: &[fairlead_guard::events::Unmet],
    ran: &[RanStep],
) -> Vec<Unverifiable> {
    let config = &planned.config;
    let mut out: Vec<Unverifiable> = not_run
        .iter()
        .map(|u| {
            let need = config.needs.items().iter().find(|n| n.id == u.need);
            let why = match need.filter(|n| !n.os.is_empty()) {
                Some(n) => {
                    let names: Vec<&str> = n.os.iter().map(|o| o.name()).collect();
                    format!("needs {}", names.join(" or "))
                }
                None => format!("needs \"{}\"", u.need),
            };
            Unverifiable {
                step: u.step.clone(),
                why,
                proved_in: need.and_then(|n| n.proved_in.clone()),
            }
        })
        .collect();
    for r in ran.iter().filter(|r| r.held) {
        let held = planned.plan.quarantined.iter().find(|q| match q.kind {
            InvocationKind::Check => q.target == r.id,
            InvocationKind::Runner => planned
                .plan
                .tests
                .iter()
                .any(|t| t.path == q.target && t.runner.as_deref() == Some(r.id.as_str())),
        });
        let step = match held {
            Some(q) if q.target != r.id => format!("{} ({})", r.id, q.target),
            _ => r.id.clone(),
        };
        out.push(Unverifiable {
            step,
            why: match held {
                Some(q) if !q.here.is_empty() => {
                    format!("held by [[quarantine]] here ({})", q.here.join(", "))
                }
                _ => "held by [[quarantine]] here".into(),
            },
            proved_in: held.map(|q| q.proved_in.clone()),
        });
    }
    out
}

fn stage_rank(stage: Option<&str>) -> usize {
    CiStage::ALL
        .iter()
        .position(|s| Some(s.name()) == stage)
        .unwrap_or(CiStage::ALL.len())
}

/// The receipt's lines: what ran, by stage and platform, then what never
/// ran on this tree and what can't be proved here.
pub fn lines(ran: &[RanStep], never: &[String], unverifiable: &[Unverifiable]) -> String {
    let mut groups: BTreeMap<(usize, String, String), Vec<String>> = BTreeMap::new();
    for r in ran {
        let platform = match (&r.os, &r.arch) {
            (Some(os), Some(arch)) => format!("{os}/{arch}"),
            (Some(os), None) => os.clone(),
            _ => "platform not recorded".into(),
        };
        let stage = r
            .stage
            .clone()
            .unwrap_or_else(|| "stage not recorded".into());
        let word = match (r.passed, r.held) {
            (true, _) => r.id.clone(),
            (false, true) => format!("{} (held)", r.id),
            (false, false) => format!("{} (failed)", r.id),
        };
        let names = groups
            .entry((stage_rank(r.stage.as_deref()), stage, platform))
            .or_default();
        if !names.contains(&word) {
            names.push(word);
        }
    }
    let mut out = String::new();
    for ((_, stage, platform), names) in groups {
        out.push_str(&format!(
            "ran      {stage}, {platform}: {}\n",
            names.join(", ")
        ));
    }
    if !never.is_empty() {
        out.push_str(&format!(
            "not run  never run on this tree: {}\n",
            never.join(", ")
        ));
    }
    for u in unverifiable {
        let at = u
            .proved_in
            .as_deref()
            .map(|p| format!("; proved in {p}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "not run  unverifiable here: {} {}{at}\n",
            u.step, u.why
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, passed: bool, stage: Option<&str>, os: Option<&str>) -> RanStep {
        RanStep {
            id: id.into(),
            passed,
            held: false,
            stage: stage.map(String::from),
            os: os.map(String::from),
            arch: os.map(|_| "x86_64".to_string()),
        }
    }

    #[test]
    fn steps_group_by_stage_then_platform_and_gaps_are_said_plainly() {
        let ran = [
            step("unit", true, Some("ready"), Some("linux")),
            step("lint", true, Some("draft"), Some("linux")),
            step("guard", false, Some("draft"), Some("linux")),
            step("old", true, None, None),
        ];
        let never = vec!["e2e".to_string()];
        let unverifiable = [Unverifiable {
            step: "build-mac".into(),
            why: "needs macos".into(),
            proved_in: Some("CI (macos job)".into()),
        }];
        assert_eq!(
            lines(&ran, &never, &unverifiable),
            "ran      draft, linux/x86_64: lint, guard (failed)\n\
             ran      ready, linux/x86_64: unit\n\
             ran      stage not recorded, platform not recorded: old\n\
             not run  never run on this tree: e2e\n\
             not run  unverifiable here: build-mac needs macos; proved in CI (macos job)\n"
        );
    }

    #[test]
    fn a_step_reads_from_an_old_event_without_where_it_ran() {
        let event: Value = serde_json::from_str(
            r#"{"steps":[{"id":"unit","passed":false,"quarantined":true,"seconds":1.0}]}"#,
        )
        .unwrap();
        let r = ran(&event);
        assert_eq!(r.len(), 1);
        assert!(r[0].held && r[0].stage.is_none() && r[0].os.is_none());
    }
}
