//! The identities a plan carries: the config's digest, the tree it was made
//! from, and a plan id stable for the same inputs.

use sha2::{Digest, Sha256};

use fairlead_core::config::Config;
use fairlead_core::plan::Change;
use fairlead_lang::cache::blob_id;
use fairlead_lang::tree::Tree;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sections that never change a plan, left out of its digest altogether, so
/// adding one doesn't change the digest of every plan made before it.
const NOT_PLANNED: [&str; 9] = [
    "guard", "hooks", "done", "brief", "ci", "memory", "skills", "agents", "stages",
];

pub fn config_digest(config: &Config) -> String {
    let mut value = serde_json::to_value(config).expect("config serializes");
    if let Some(map) = value.as_object_mut() {
        for key in NOT_PLANNED {
            map.remove(key);
        }
        // A step's `from` says when it runs, not what it proves, so moving it
        // keeps a tree's reuse proof and the base of a since-green plan.
        let drop_from = |list: Option<&mut serde_json::Value>| {
            let steps = list.and_then(|l| l.as_array_mut()).into_iter().flatten();
            for step in steps.filter_map(|s| s.as_object_mut()) {
                step.remove("from");
            }
        };
        drop_from(map.get_mut("tests").and_then(|t| t.get_mut("runners")));
        drop_from(map.get_mut("checks"));
    }
    let json = serde_json::to_vec(&value).expect("config serializes");
    format!("sha256:{}", hex(&Sha256::digest(json)))
}

/// A hash of every file's path and blob id, for a working tree that doesn't
/// match any commit.
pub fn worktree_hash(tree: &Tree) -> String {
    let mut hasher = Sha256::new();
    for file in &tree.files {
        let bytes = std::fs::read(tree.abs(file)).unwrap_or_default();
        hasher.update(file.as_bytes());
        hasher.update([0]);
        hasher.update(blob_id(&bytes).as_bytes());
        hasher.update([b'\n']);
    }
    format!("worktree:{}", hex(&hasher.finalize()))
}

pub fn plan_id(
    config_digest: &str,
    tree_hash: &str,
    base: Option<&str>,
    changes: &[Change],
) -> String {
    let mut hasher = Sha256::new();
    let version = env!("CARGO_PKG_VERSION");
    for part in [version, config_digest, tree_hash, base.unwrap_or("")] {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hasher.update(serde_json::to_vec(changes).expect("changes serialize"));
    format!("pl_{}", &hex(&hasher.finalize())[..16])
}

/// A plan id for the same inputs cut down to one CI stage, so a draft plan
/// and a ready plan of one tree never share an id.
pub fn with_stage(plan_id: &str, stage: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plan_id.as_bytes());
    hasher.update([0]);
    hasher.update(stage.as_bytes());
    format!("pl_{}", &hex(&hasher.finalize())[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_rules_leave_the_config_digest_alone() {
        let mut config = Config::default();
        let before = config_digest(&config);
        config.guard.baseline = "other.json".into();
        config.hooks.claude = fairlead_core::config::HooksTarget::Local;
        assert_eq!(config_digest(&config), before);
        config.tests.unreached = fairlead_core::config::Unreached::All;
        assert_ne!(config_digest(&config), before);
    }

    #[test]
    fn the_default_config_keeps_the_digest_0_5_1_gave_it() {
        let mut config = Config::default();
        let released = "sha256:7eff5b583b350526d891c873a741620ab5faab28c4f4dcece469b1a327dc29d3";
        assert_eq!(config_digest(&config), released);
        config.brief.nudge = false;
        config.ci.comment = true;
        config.done.guard = false;
        config.agents.write = fairlead_core::config::AgentsWrite::Never;
        config.agents.files = vec!["AGENTS.md".to_string()].into();
        assert_eq!(
            config_digest(&config),
            released,
            "sections that never change a plan"
        );
    }

    #[test]
    fn stages_and_a_steps_from_leave_the_digest_alone() {
        use fairlead_core::config::{CiStage, Runner, Stages};
        let runner = |from| Runner {
            id: "e2e".into(),
            matches: vec!["e2e/**".into()],
            exclude: Vec::new(),
            invoke: Default::default(),
            cwd: None,
            command: vec!["playwright".into(), "test".into()],
            all_command: None,
            exclude_arg: None,
            from,
        };
        let mut config = Config::default();
        config.tests.runners = vec![runner(None)].into();
        let before = config_digest(&config);
        config.tests.runners = vec![runner(Some(CiStage::Merge))].into();
        config.stages = Some(Stages::default());
        assert_eq!(config_digest(&config), before);
        assert_eq!(
            config_digest(&Config::default()),
            "sha256:7eff5b583b350526d891c873a741620ab5faab28c4f4dcece469b1a327dc29d3"
        );
    }

    #[test]
    fn a_stage_gives_a_plan_its_own_id() {
        let id = plan_id("sha256:x", "t", None, &[]);
        assert_ne!(with_stage(&id, "draft"), with_stage(&id, "ready"));
        assert_eq!(with_stage(&id, "draft"), with_stage(&id, "draft"));
        assert!(with_stage(&id, "draft").starts_with("pl_"));
    }

    #[test]
    fn no_providers_keeps_the_digest_a_config_had_before_they_existed() {
        let config = Config::default();
        let json = serde_json::to_value(&config).unwrap();
        assert!(json["graph"].get("providers").is_none(), "{json}");
        let mut with = config.clone();
        with.graph.providers = vec![fairlead_core::config::GraphProvider {
            id: "go".into(),
            command: vec!["go-graph".into()],
            files: vec!["**/*.go".into()],
            timeout_seconds: 120,
        }]
        .into();
        assert_ne!(config_digest(&with), config_digest(&config));
    }
}
