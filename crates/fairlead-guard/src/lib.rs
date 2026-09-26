//! Project rules, checked at three stages by one engine: as an agent writes a
//! file, at commit, and in CI. A rule reads one file and reports findings;
//! the stage decides what a finding costs.

pub mod added;
pub mod baseline;
pub mod events;
mod finding;
pub mod git;
mod rules;
mod size;

pub use finding::{sort, Finding, Measure};
pub use rules::{Guard, Rule, Source};
