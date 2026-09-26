//! Test selection: the modules a repository is split into, its test files
//! and their runners and classes, owner rules, and the plan that says which
//! tests and checks a change can affect, each with the reason it's in.

pub mod checks;
pub mod digest;
pub mod explain;
pub mod git;
pub mod invoke;
pub mod lockfile;
pub mod modules;
pub mod owners;
pub mod pattern;
pub mod planner;
pub mod render;
pub mod select;
pub mod testfiles;
pub mod walk;

pub use planner::{plan, Input};
