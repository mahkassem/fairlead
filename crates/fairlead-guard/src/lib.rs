//! Project rules, checked at three stages by one engine: as an agent writes a
//! file, at commit, and in CI. A rule reads one file and reports findings;
//! the stage decides what a finding costs.

pub mod added;
pub mod baseline;
mod citations;
pub mod commands;
mod comments;
pub mod events;
pub mod external;
mod finding;
mod functions;
pub mod git;
pub mod migrations;
mod names;
mod rules;
mod size;
pub mod stages;

pub use finding::{sort, Finding, Measure};
pub use rules::{Guard, Preset, Source};
