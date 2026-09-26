//! The plan's JSON Schema is a public contract: the committed copy must
//! match what the types generate. `UPDATE_SCHEMA=1 cargo test` rewrites it.

use std::path::PathBuf;

#[test]
fn the_committed_plan_schema_matches_the_types() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/src/plan-v1.schema.json");
    let generated = format!(
        "{}\n",
        serde_json::to_string_pretty(&fairlead_core::plan::json_schema()).unwrap()
    );
    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        committed == generated,
        "docs/src/plan-v1.schema.json is out of date; if the change is additive, rerun with UPDATE_SCHEMA=1, otherwise bump the plan version"
    );
}
