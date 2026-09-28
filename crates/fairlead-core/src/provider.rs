//! What an external graph provider prints: a public contract, versioned
//! like the plan, so any stack can plug in without changing Fairlead.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderOutput {
    /// Always 1 for this shape.
    pub version: u32,
    /// Dependencies among repo-relative paths: `from` depends on `to`.
    pub edges: Vec<ProviderEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderEdge {
    pub from: String,
    pub to: String,
}

pub fn json_schema() -> serde_json::Value {
    crate::plan::sorted(
        serde_json::to_value(schemars::schema_for!(ProviderOutput)).expect("schema serializes"),
    )
}
