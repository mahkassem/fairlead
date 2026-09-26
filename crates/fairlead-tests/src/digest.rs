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

/// The guard's rules don't change a plan, so they don't change its digest.
pub fn config_digest(config: &Config) -> String {
    let planned = Config {
        guard: Default::default(),
        ..config.clone()
    };
    let json = serde_json::to_vec(&planned).expect("config serializes");
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
        assert_eq!(config_digest(&config), before);
        config.tests.unreached = fairlead_core::config::Unreached::All;
        assert_ne!(config_digest(&config), before);
    }
}
