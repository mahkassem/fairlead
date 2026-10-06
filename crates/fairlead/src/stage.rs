//! CI stages for `ci plan`: which stage this run is, read from the GitHub
//! event, and the plan cut down to the runners and checks that run at it.

use std::collections::BTreeMap;

use clap::ValueEnum;
use fairlead_core::config::{CiStage, Config, Stages};
use fairlead_core::pattern::Pattern;
use fairlead_core::plan::{Deferred, InvocationKind, Plan};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum StageArg {
    /// Read it from the GitHub event.
    Auto,
    /// No stage: plan as before stages existed.
    None,
    Draft,
    Ready,
    Merge,
    Full,
}

/// The stage a run is cut down to, if any, and why, for the log.
#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
    pub stage: Option<CiStage>,
    pub why: String,
}

/// What GitHub Actions says about the run that started this one.
pub struct Event {
    pub name: String,
    pub git_ref: String,
    pub payload: Option<Value>,
}

impl Event {
    pub fn from_env() -> Option<Event> {
        let name = std::env::var("GITHUB_EVENT_NAME")
            .ok()
            .filter(|n| !n.is_empty())?;
        let payload = std::env::var_os("GITHUB_EVENT_PATH")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok());
        Some(Event {
            name,
            git_ref: std::env::var("GITHUB_REF").unwrap_or_default(),
            payload,
        })
    }
}

/// The stage for `--stage`, or when it's absent, for a config that uses
/// stages; a config that doesn't plans as it always has.
pub fn resolve(arg: Option<StageArg>, config: &Config, event: Option<&Event>) -> Resolved {
    let fixed = |stage, why: &str| Resolved {
        stage: Some(stage),
        why: why.into(),
    };
    match arg {
        Some(StageArg::None) => Resolved {
            stage: None,
            why: "--stage none".into(),
        },
        Some(StageArg::Draft) => fixed(CiStage::Draft, "--stage"),
        Some(StageArg::Ready) => fixed(CiStage::Ready, "--stage"),
        Some(StageArg::Merge) => fixed(CiStage::Merge, "--stage"),
        Some(StageArg::Full) => fixed(CiStage::Full, "--stage"),
        None if !config.uses_stages() => Resolved {
            stage: None,
            why: "the config sets no stages".into(),
        },
        Some(StageArg::Auto) | None => {
            let (stage, why) = match event {
                Some(e) => from_event(e, &config.stages_or_default()),
                None => (
                    CiStage::Ready,
                    "not in GitHub Actions, so ready".to_string(),
                ),
            };
            Resolved {
                stage: Some(stage),
                why: format!("auto: {why}"),
            }
        }
    }
}

fn is_environment(branch: &str, stages: &Stages) -> bool {
    stages
        .environments
        .items()
        .iter()
        .any(|env| env == branch || Pattern::new(env).is_ok_and(|p| p.is_match(branch)))
}

/// The stage an event is, and the reason in a few words.
pub fn from_event(event: &Event, stages: &Stages) -> (CiStage, String) {
    let pr = event.payload.as_ref().and_then(|p| p.get("pull_request"));
    let labelled = pr
        .and_then(|p| p.get("labels"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|l| l.get("name").and_then(Value::as_str) == Some(stages.full_label.as_str()));
    let name = event.name.as_str();
    match name {
        "pull_request" | "pull_request_target" if labelled => (
            CiStage::Full,
            format!("{name} labelled {}", stages.full_label),
        ),
        "pull_request" | "pull_request_target" => {
            if pr.and_then(|p| p.get("draft")).and_then(Value::as_bool) == Some(true) {
                (CiStage::Draft, format!("{name}, a draft"))
            } else {
                (CiStage::Ready, format!("{name}, not a draft"))
            }
        }
        "merge_group" => (CiStage::Merge, "merge_group".into()),
        "push" => {
            let branch = event.git_ref.strip_prefix("refs/heads/");
            match branch {
                Some(b) if is_environment(b, stages) => {
                    (CiStage::Merge, format!("push to {b}, an environment"))
                }
                Some(b) => (CiStage::Ready, format!("push to {b}, not an environment")),
                None => (CiStage::Ready, format!("push to {}", event.git_ref)),
            }
        }
        "schedule" | "workflow_dispatch" => (CiStage::Full, name.to_string()),
        other => (CiStage::Ready, format!("{other}, so ready")),
    }
}

/// When a configured runner or check runs: its `from`, or its kind's default.
pub fn from_of(config: &Config, id: &str, kind: &InvocationKind) -> CiStage {
    match kind {
        InvocationKind::Runner => config
            .tests
            .runners
            .items()
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.from)
            .unwrap_or(CiStage::Ready),
        InvocationKind::Check => config
            .checks
            .items()
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.from)
            .unwrap_or(CiStage::Draft),
    }
}

/// Cut `plan` down to what runs at `stage`: a later step's invocations,
/// tests and checks move to `deferred`, and the plan gets an id of its own.
pub fn apply(plan: &mut Plan, config: &Config, stage: CiStage) {
    let later = |id: &str, kind: &InvocationKind| from_of(config, id, kind) > stage;
    let mut deferred: Vec<Deferred> = Vec::new();
    let mut defer = |id: &str, kind: InvocationKind, count: usize| match deferred
        .iter_mut()
        .find(|d| d.id == id && d.kind == kind)
    {
        Some(d) => d.selected += count,
        None => deferred.push(Deferred {
            id: id.to_string(),
            from: from_of(config, id, &kind),
            kind,
            selected: count,
        }),
    };
    plan.tests.retain(|t| match &t.runner {
        Some(r) if later(r, &InvocationKind::Runner) => {
            defer(r, InvocationKind::Runner, 1);
            false
        }
        _ => true,
    });
    plan.checks.retain(|c| {
        let keep = !later(&c.id, &InvocationKind::Check);
        if !keep {
            defer(&c.id, InvocationKind::Check, 1);
        }
        keep
    });
    for inv in plan.invocations.iter().filter(|i| later(&i.id, &i.kind)) {
        defer(&inv.id, inv.kind.clone(), 0);
    }
    plan.invocations.retain(|i| !later(&i.id, &i.kind));
    let (tests, checks) = (&plan.tests, &plan.checks);
    plan.quarantined.retain(|q| match q.kind {
        InvocationKind::Runner => tests.iter().any(|t| t.path == q.target),
        InvocationKind::Check => checks.iter().any(|c| c.id == q.target),
    });
    plan.deferred = deferred;
    plan.stage = Some(stage);
    plan.plan_id = fairlead_tests::digest::with_stage(&plan.plan_id, stage.name());
}

/// A step id as a GitHub output name: anything but letters, digits and `_` becomes `_`.
pub fn output_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("run_{safe}")
}

/// `run_<id>` for every configured runner and check: whether this plan runs
/// it. Two ids that become one output name are an error, not a guess.
pub fn run_outputs(plan: &Plan, config: &Config) -> Result<Vec<(String, bool)>, String> {
    let ids = config
        .tests
        .runners
        .items()
        .iter()
        .map(|r| r.id.as_str())
        .chain(config.checks.items().iter().map(|c| c.id.as_str()));
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    let mut out = Vec::new();
    for id in ids {
        let name = output_name(id);
        if let Some(other) = seen.insert(name.clone(), id) {
            if other != id {
                return Err(format!(
                    "`{other}` and `{id}` both become the output {name}; rename one"
                ));
            }
            continue;
        }
        out.push((name, plan.invocations.iter().any(|i| i.id == id)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(name: &str, git_ref: &str, payload: Value) -> Event {
        Event {
            name: name.into(),
            git_ref: git_ref.into(),
            payload: Some(payload),
        }
    }

    fn stage_of(e: &Event) -> CiStage {
        from_event(e, &Stages::default()).0
    }

    #[test]
    fn each_event_maps_to_its_stage() {
        let pr = |draft: bool, labels: Value| json!({"pull_request": {"draft": draft, "labels": labels}});
        let cases = [
            (
                event("pull_request", "refs/pull/1/merge", pr(true, json!([]))),
                CiStage::Draft,
            ),
            (
                event("pull_request", "refs/pull/1/merge", pr(false, json!([]))),
                CiStage::Ready,
            ),
            (
                event(
                    "pull_request_target",
                    "refs/heads/main",
                    pr(true, json!([])),
                ),
                CiStage::Draft,
            ),
            (
                event(
                    "pull_request",
                    "refs/pull/1/merge",
                    pr(true, json!([{"name": "run-everything"}])),
                ),
                CiStage::Full,
            ),
            (event("push", "refs/heads/main", json!({})), CiStage::Merge),
            (
                event("push", "refs/heads/feature/x", json!({})),
                CiStage::Ready,
            ),
            (event("push", "refs/tags/v1.0.0", json!({})), CiStage::Ready),
            (
                event(
                    "merge_group",
                    "refs/heads/gh-readonly-queue/main/x",
                    json!({}),
                ),
                CiStage::Merge,
            ),
            (
                event("schedule", "refs/heads/main", json!({})),
                CiStage::Full,
            ),
            (
                event("workflow_dispatch", "refs/heads/feature/x", json!({})),
                CiStage::Full,
            ),
            (
                event("workflow_run", "refs/heads/main", json!({})),
                CiStage::Ready,
            ),
        ];
        for (e, want) in cases {
            assert_eq!(stage_of(&e), want, "{} {}", e.name, e.git_ref);
        }
    }

    #[test]
    fn environments_are_names_or_globs() {
        let stages = Stages {
            environments: vec!["main".to_string(), "release/*".to_string()].into(),
            ..Stages::default()
        };
        let push = |r: &str| from_event(&event("push", r, json!({})), &stages).0;
        assert_eq!(push("refs/heads/release/2026-10"), CiStage::Merge);
        assert_eq!(push("refs/heads/staging"), CiStage::Ready);
    }

    #[test]
    fn no_stage_unless_asked_or_configured() {
        let mut config = Config::default();
        let push = event("push", "refs/heads/main", json!({}));
        assert_eq!(resolve(None, &config, Some(&push)).stage, None);
        assert_eq!(
            resolve(Some(StageArg::Auto), &config, Some(&push)).stage,
            Some(CiStage::Merge)
        );
        config.stages = Some(Stages::default());
        let auto = resolve(None, &config, Some(&push));
        assert_eq!(auto.stage, Some(CiStage::Merge));
        assert_eq!(auto.why, "auto: push to main, an environment");
        assert_eq!(
            resolve(Some(StageArg::None), &config, Some(&push)).stage,
            None
        );
        assert_eq!(
            resolve(None, &config, None),
            Resolved {
                stage: Some(CiStage::Ready),
                why: "auto: not in GitHub Actions, so ready".into()
            }
        );
    }

    #[test]
    fn output_names_are_safe_and_a_collision_is_an_error() {
        assert_eq!(output_name("desktop-build"), "run_desktop_build");
        let mut config = Config::default();
        let check = |id: &str| fairlead_core::config::Check {
            id: id.into(),
            command: vec!["true".into()],
            paths: Vec::new(),
            modules: Vec::new(),
            files: None,
            from: None,
        };
        config.checks = vec![check("e2e-web"), check("e2e_web")].into();
        let plan: Plan = serde_json::from_value(json!({
            "version": 1, "plan_id": "pl_x", "fairlead_version": "0", "config_digest": "d",
            "tree_hash": "t", "head": "h", "all": false, "changed": [], "ignored": [],
            "tests": [], "checks": [], "invocations": [], "unreached": [], "warnings": []
        }))
        .unwrap();
        let err = run_outputs(&plan, &config).unwrap_err();
        assert!(err.contains("`e2e-web` and `e2e_web`"), "{err}");
    }
}
