//! What the done gate's record says about a tree, in the words `done`,
//! `done --check`, `next` and the Stop hook share: what isn't finished, the
//! rule that holds it, and the way forward.

use std::path::Path;

use fairlead_core::config::{Config, Phase};
use fairlead_guard::events::Waiting;
use serde_json::Value;

/// The newest `done` event for `tree`, if one ran.
pub fn last_done(log: &str, tree: &str) -> Option<Value> {
    log.lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|e| e["stage"] == "done" && e["tree"] == tree)
}

/// Where a tree stands with the gate.
#[derive(Debug, PartialEq)]
pub enum State {
    Passed,
    /// Its steps passed, and whole suites wait for `done --full`.
    Waits(Vec<Waiting>),
    /// A need isn't met here and names no `proved_in`, so what it covers
    /// in this step wasn't run.
    Unmet {
        need: String,
        step: String,
    },
    /// Failed, at this step when the record names one.
    Failed(Option<String>),
    NotRun,
}

pub fn state(event: Option<&Value>) -> State {
    let Some(e) = event else {
        return State::NotRun;
    };
    match e["decision"].as_str() {
        Some("pass") => State::Passed,
        Some("deferred") => {
            State::Waits(serde_json::from_value(e["waiting"].clone()).unwrap_or_default())
        }
        _ if unproved(e).is_some() => {
            let u = unproved(e).expect("checked");
            State::Unmet {
                need: u["need"].as_str().unwrap_or("").to_string(),
                step: u["step"].as_str().unwrap_or("").to_string(),
            }
        }
        _ => State::Failed(
            e["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["passed"] == false && s["quarantined"] != true)
                .and_then(|s| s["id"].as_str().map(String::from)),
        ),
    }
}

/// The first unmet need a failed run's record names with no `proved_in`,
/// unless a step itself failed first.
fn unproved(e: &Value) -> Option<&Value> {
    let failed = e["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| s["passed"] == false && s["quarantined"] != true);
    if failed {
        return None;
    }
    e["unmet"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|u| u["proved"] == false)
}

/// Why a step wasn't run, and the two ways forward.
pub fn unmet_line(need: &str, step: &str) -> String {
    format!(
        "`{step}` wasn't run here because need \"{need}\" isn't met and names no `proved_in`; meet it (`fairlead doctor` checks it) or set its `proved_in` to where it's proved, then run `fairlead done` again"
    )
}

/// Why whole suites hold the gate open, and the ways forward: the phase
/// that holds them, what the change reaches of each, and the scheduled
/// workflow that runs them when one does.
pub fn hold(waiting: &[Waiting], config: &Config, root: &Path) -> String {
    let ids = match waiting.len() {
        0 => "a whole suite".to_string(),
        _ => waiting
            .iter()
            .map(|w| w.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    };
    let reach = |w: &Waiting| {
        let rule = match w.by_rule {
            0 => String::new(),
            n => format!(", {n} through a `[[graph.edges]]` rule"),
        };
        format!("{} of {} test files{rule}", w.reached, w.selected)
    };
    let counts = match waiting {
        [] => String::new(),
        [one] => format!(" ({})", reach(one)),
        many => {
            let each: Vec<String> = many
                .iter()
                .map(|w| format!("{}: {}", w.id, reach(w)))
                .collect();
            format!(" ({})", each.join("; "))
        }
    };
    let (holder, because) = match config.phase {
        Some(Phase::Testing) => (
            "`phase = \"testing\"`",
            ", because testers use the deployed build",
        ),
        Some(Phase::Early) => ("`phase = \"early\"`", ""),
        Some(Phase::Live) => (
            "`phase = \"live\"`",
            ", because people use the deployed build",
        ),
        None => ("a project with no `phase`", ", since it counts as live"),
    };
    let found = crate::phase::workflows(root);
    let early = match crate::phase::scheduled_workflow(&found) {
        Some(file) => format!("let the scheduled run ({file}) prove it"),
        None => "add a scheduled run with `fairlead ci workflow --write`".to_string(),
    };
    let alternative = match config.phase {
        Some(Phase::Live) => String::new(),
        _ => format!(
            ", or, if nobody outside the team uses a deployed build, set `phase = \"early\"` and {early}"
        ),
    };
    format!(
        "{ids} waits: {holder} holds a change that reaches most of a waiting suite{counts} until `fairlead done --full` passes on this tree{because}. Run `fairlead done --full` here{alternative}."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_reads_as_a_pass_a_wait_with_its_suites_or_a_failed_step() {
        let log = [
            r#"{"stage":"done","tree":"t1","decision":"fail","steps":[{"id":"unit","passed":false}]}"#,
            r#"{"stage":"done","tree":"t2","decision":"deferred","waiting":[{"id":"e2e","reached":3,"selected":4,"by_rule":2}]}"#,
            r#"{"stage":"done","tree":"t3","decision":"pass"}"#,
        ]
        .join("\n");
        let at = |t| state(last_done(&log, t).as_ref());
        assert_eq!(at("t1"), State::Failed(Some("unit".into())));
        assert_eq!(at("t3"), State::Passed);
        assert_eq!(at("t4"), State::NotRun);
        let State::Waits(waiting) = at("t2") else {
            panic!("a deferred run waits");
        };
        let config = Config {
            phase: Some(Phase::Testing),
            ..Config::default()
        };
        let text = hold(&waiting, &config, Path::new("/nonexistent"));
        assert_eq!(
            text,
            "e2e waits: `phase = \"testing\"` holds a change that reaches most of a waiting suite (3 of 4 test files, 2 through a `[[graph.edges]]` rule) until `fairlead done --full` passes on this tree, because testers use the deployed build. Run `fairlead done --full` here, or, if nobody outside the team uses a deployed build, set `phase = \"early\"` and add a scheduled run with `fairlead ci workflow --write`."
        );
    }
}
