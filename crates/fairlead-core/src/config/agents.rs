//! `[agents]`: the marked block `fairlead agents sync` keeps in the files
//! agents read first.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{strings, List};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Agents {
    /// Whether `agents sync` writes the block, or only prints it.
    pub write: AgentsWrite,
    /// The files that carry the block, from the repository root.
    pub files: List<String>,
}

impl Default for Agents {
    fn default() -> Self {
        Agents {
            write: AgentsWrite::Block,
            files: strings(&["AGENTS.md", "CLAUDE.md"]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AgentsWrite {
    /// Keep the block between the markers, and nothing else in the file.
    #[default]
    Block,
    /// Write nothing: print the block for a person to copy in.
    Never,
}
