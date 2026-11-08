//! The project's phase in `fairlead doctor`: what the workflows say about a
//! live deployment, whether that agrees with `phase`, and whether anything
//! runs the whole suites a plan leaves to the full stage.

use std::path::Path;

use fairlead_core::config::{CiStage, Config};

/// Text in a workflow that deploys something people use. A hint only: the
/// config's `phase` decides.
const LIVE: [&str; 6] = [
    "--prod",
    "helm upgrade",
    "argocd app sync",
    "fly deploy",
    "firebase deploy",
    "kubectl apply",
];

/// GitHub environment names that mean people use what's deployed.
const LIVE_ENVIRONMENTS: [&str; 3] = ["production", "prod", "live"];

/// The environment a job line names, in `environment: X` or, on the line
/// after a bare `environment:`, `name: X`.
fn environment_name(line: &str, after_bare: bool) -> Option<&str> {
    let value = match line.strip_prefix("environment:") {
        Some(v) => v,
        None if after_bare => line.strip_prefix("name:")?,
        None => return None,
    };
    let value = value.trim().trim_matches(|c| c == '\'' || c == '"');
    (!value.is_empty()).then_some(value)
}

/// A workflow file's name and text.
pub struct Workflow {
    pub name: String,
    pub text: String,
}

pub fn workflows(root: &Path) -> Vec<Workflow> {
    let dir = root.join(".github/workflows");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<Workflow> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".yml") || name.ends_with(".yaml")
        })
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            Some(Workflow {
                name: format!(".github/workflows/{}", e.file_name().to_string_lossy()),
                text,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Where a workflow seems to deploy to people, as `file:line`.
pub fn live_evidence(workflows: &[Workflow]) -> Vec<String> {
    let mut out = Vec::new();
    for w in workflows {
        let mut after_bare = false;
        for (n, line) in w.text.lines().enumerate() {
            let line = line.trim().to_ascii_lowercase();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let env = environment_name(&line, after_bare);
            after_bare = line == "environment:";
            let live_env = env.is_some_and(|e| LIVE_ENVIRONMENTS.contains(&e));
            if live_env || LIVE.iter().any(|m| line.contains(m)) {
                out.push(format!("{}:{}", w.name, n + 1));
                break;
            }
        }
    }
    out
}

/// Whether a workflow on a schedule runs Fairlead's CI plan, which plans
/// the full stage when the schedule starts it.
pub fn scheduled_plan(workflows: &[Workflow]) -> bool {
    workflows.iter().any(|w| {
        let lines: Vec<&str> = w.text.lines().map(str::trim).collect();
        lines.iter().any(|l| l.starts_with("schedule:"))
            && lines.iter().any(|l| l.contains("fairlead ci plan"))
    })
}

/// Runners and checks whose whole suite runs only at the full stage.
fn at_full(config: &Config) -> Vec<String> {
    let runners = config
        .tests
        .runners
        .items()
        .iter()
        .map(|r| (&r.id, r.all_from));
    let checks = config.checks.items().iter().map(|c| (&c.id, c.all_from));
    runners
        .chain(checks)
        .filter(|(_, set)| config.all_from_of(*set) == Some(CiStage::Full))
        .map(|(id, _)| id.clone())
        .collect()
}

/// The `phase` lines of `fairlead doctor`.
pub fn doctor_lines(root: &Path, config: &Config) -> String {
    let found = workflows(root);
    let live = live_evidence(&found);
    let mut out = match (config.phase, live.first()) {
        (Some(p), Some(at)) if !p.is_live() => format!(
            "phase: {} (set), but {at} looks like a deploy to people; once one is live, set `phase = \"live\"`\n",
            p.name()
        ),
        (Some(p), _) => format!("phase: {} (set)\n", p.name()),
        (None, Some(at)) => format!(
            "phase: not set; {at} looks like a deploy to people, so whole suites run as each step says (`phase = \"live\"` says so)\n"
        ),
        (None, None) => "phase: not set, and no workflow deploys to people; if nothing is live, `phase = \"early\"` leaves whole suites to the end of the day\n".to_string(),
    };
    let waiting = at_full(config);
    if !waiting.is_empty() && !scheduled_plan(&found) {
        out.push_str(&format!(
            "  whole suites of {} run only at the full stage, and no scheduled workflow runs `fairlead ci plan`: `fairlead ci workflow --write` adds the schedule\n",
            waiting.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fairlead_core::config::Phase;

    fn wf(name: &str, text: &str) -> Workflow {
        Workflow {
            name: name.into(),
            text: text.into(),
        }
    }

    #[test]
    fn a_production_environment_is_live_evidence_and_a_comment_isnt() {
        let found = [
            wf("a.yml", "jobs:\n  ship:\n    environment: production\n"),
            wf("b.yml", "# environment: production\nrun: echo\n"),
        ];
        assert_eq!(live_evidence(&found), vec!["a.yml:3".to_string()]);
    }

    #[test]
    fn an_environment_named_on_its_own_line_is_live_evidence_and_staging_isnt() {
        let found = [
            wf(
                "prod.yml",
                "jobs:\n  ship:\n    environment:\n      name: production\n      url: https://x\n",
            ),
            wf("quoted.yml", "jobs:\n  ship:\n    environment: 'live'\n"),
            wf(
                "stage.yml",
                "jobs:\n  ship:\n    environment:\n      name: staging\n    steps:\n      - name: production smoke\n",
            ),
        ];
        assert_eq!(
            live_evidence(&found),
            vec!["prod.yml:4".to_string(), "quoted.yml:3".to_string()]
        );
    }

    #[test]
    fn a_schedule_counts_only_with_a_ci_plan() {
        let nightly = wf("n.yml", "on:\n  schedule:\n    - cron: \"1 2 * * *\"\nsteps:\n  - run: fairlead ci plan --out p.json\n");
        let insights = wf(
            "i.yml",
            "on:\n  schedule:\n    - cron: \"1 2 * * 1\"\nsteps:\n  - run: fairlead insights\n",
        );
        assert!(scheduled_plan(&[nightly]));
        assert!(!scheduled_plan(&[insights]));
    }

    #[test]
    fn early_with_a_deploy_says_to_set_live() {
        let dir = std::env::temp_dir().join(format!("fl-phase-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        std::fs::write(
            dir.join(".github/workflows/ship.yml"),
            "jobs:\n  ship:\n    steps:\n      - run: helm upgrade api ./chart\n",
        )
        .unwrap();
        let config = Config {
            phase: Some(Phase::Early),
            ..Config::default()
        };
        let text = doctor_lines(&dir, &config);
        assert!(
            text.contains("phase: early (set), but .github/workflows/ship.yml:4"),
            "{text}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
