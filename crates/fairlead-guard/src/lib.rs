//! Project rules, checked at three stages by one engine: as an agent writes a
//! file, at commit, and in CI. A rule reads one file and reports findings;
//! the stage decides what a finding costs.

pub mod added;
pub mod baseline;
mod citations;
pub mod claims;
pub mod commands;
mod comments;
pub mod edit;
pub mod events;
pub mod external;
pub mod fetch;
mod finding;
mod functions;
pub mod git;
pub mod gitleaks;
pub mod head_paths;
pub mod hook;
pub mod lefthook;
pub mod migrations;
mod names;
pub mod patch;
pub mod pr;
mod rules;
pub mod secret_paths;
mod size;
pub mod stages;
pub mod summary;

pub use finding::{sort, Finding, Measure};
pub use rules::{Guard, Preset, Source};
