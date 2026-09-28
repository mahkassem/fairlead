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
const NOT_PLANNED: [&str; 3] = ["guard", "hooks", "done"];

pub fn config_digest(config: &Config) -> String {
    let mut value = serde_json::to_value(config).expect("config serializes");
    if let Some(map) = value.as_object_mut() {
        for key in NOT_PLANNED {
            map.remove(key);
        }
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
