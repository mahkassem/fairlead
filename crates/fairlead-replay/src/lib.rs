//! Replay: re-plan real failed CI runs and count what a plan would have
//! missed. Fetching needs the network and runs in CI; extraction,
//! attribution and the report are plain functions over recorded data.

pub mod attribute;
pub mod dataset;
pub mod extract;
pub mod fetch;
pub mod git;
pub mod github;
pub mod gotest;
pub mod inherited;
pub mod jvm;
pub mod phpunit;
pub mod quarantine;
pub mod recurring;
pub mod report;
pub mod run;
pub mod waves;
pub mod window;
