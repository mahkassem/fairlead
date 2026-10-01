//! `[[quarantine]]`: a test or check that fails on one platform whatever the
//! change, held there with its evidence, so a failure it is known to give is
//! reported as not provable here instead of as a regression.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A test file or check held on the OS and under the conditions it names.
/// There it runs on its own, and a failure whose output matches `signature`
/// is not provable here; any other failure, and any failure after `until`,
/// counts. Elsewhere the entry does nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformQuarantine {
    /// The test file, exactly as the repository names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Or a `[[checks]]` id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    /// The OS it fails on; any OS when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<Os>,
    /// Conditions detected on the machine, all of which must hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Condition>,
    /// A regex over the failure's output: the failure the entry expects.
    pub signature: String,
    /// The evidence, in a sentence.
    pub reason: String,
    /// Where it is proved instead, such as "CI on Linux".
    pub proved_in: String,
    /// The last day the entry applies, `YYYY-MM-DD`.
    pub until: String,
}

impl PlatformQuarantine {
    /// The test file or check id, as the plan names it.
    pub fn target(&self) -> &str {
        self.path.as_deref().or(self.check.as_deref()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Os {
    Windows,
    Macos,
    Linux,
}

impl Os {
    pub fn name(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Macos => "macos",
            Os::Linux => "linux",
        }
    }
}

/// What makes a test lie, detected on the machine rather than declared.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Condition {
    /// `core.autocrlf` is true, so text files are checked out with CRLF.
    Autocrlf,
    /// The repository's path has a space in it.
    SpaceInPath,
}

impl Condition {
    pub fn name(self) -> &'static str {
        match self {
            Condition::Autocrlf => "autocrlf",
            Condition::SpaceInPath => "space-in-path",
        }
    }
}
