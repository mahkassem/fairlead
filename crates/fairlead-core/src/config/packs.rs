//! Framework packs: bundles of graph rules a config names in `extends`. A
//! built-in pack ships in the binary, so it moves with the release; a path
//! names a pack file the project keeps. Each is a layer under the project's
//! own files, and may set only rules, barriers and `plan.run_all`.

use std::path::Path;

use serde_json::Value;

use super::load::{error, read_layer, ConfigError};

/// The built-in packs, by name.
pub const BUILTIN: &[(&str, &str)] = &[("laravel", include_str!("../../packs/laravel.toml"))];

const ALLOWED: [(&str, &[&str]); 2] = [("graph", &["edges", "barrier"]), ("plan", &["run_all"])];

/// The layers the `extends` lists of `files` name, in order, each once.
pub fn layers(files: &[(String, Value)], root: &Path) -> Result<Vec<(String, Value)>, ConfigError> {
    let mut names: Vec<(String, String)> = Vec::new();
    for (label, value) in files {
        let Some(list) = value.get("extends").and_then(Value::as_array) else {
            continue;
        };
        for name in list.iter().filter_map(Value::as_str) {
            if !names.iter().any(|(n, _)| n == name) {
                names.push((name.to_string(), label.clone()));
            }
        }
    }
    names
        .into_iter()
        .map(|(name, from)| {
            let label = format!("pack {name}");
            let value = pack(&name, &from, root)?;
            allowed(&label, &value)?;
            Ok((label, value))
        })
        .collect()
}

fn pack(name: &str, from: &str, root: &Path) -> Result<Value, ConfigError> {
    if let Some((_, text)) = BUILTIN.iter().find(|(n, _)| *n == name) {
        return toml::from_str(text)
            .map_err(|e| error(&format!("pack {name}"), None, e.to_string()));
    }
    let path = root.join(name);
    let is_file = [".toml", ".yaml", ".yml"]
        .iter()
        .any(|ext| name.ends_with(ext));
    if !is_file || !path.is_file() {
        let known: Vec<&str> = BUILTIN.iter().map(|(n, _)| *n).collect();
        return Err(error(
            from,
            Some("extends".into()),
            format!(
                "`{name}` is neither a built-in pack ({}) nor a pack file under the project root",
                known.join(", ")
            ),
        ));
    }
    read_layer(&path)
}

/// A pack sets only what packs are for, so it can't change how a project runs its tests.
fn allowed(label: &str, value: &Value) -> Result<(), ConfigError> {
    let Some(map) = value.as_object() else {
        return Ok(());
    };
    for (table, inner) in map {
        let keys = ALLOWED.iter().find(|(t, _)| t == table).map(|(_, k)| *k);
        let bad = match (keys, inner.as_object()) {
            (Some(keys), Some(inner)) => inner
                .keys()
                .find(|k| !keys.contains(&k.as_str()))
                .map(|k| format!("{table}.{k}")),
            _ => Some(table.clone()),
        };
        if let Some(key) = bad {
            return Err(error(
                label,
                Some(key.clone()),
                format!(
                    "a pack may set only graph.edges, graph.barrier and plan.run_all, not `{key}`"
                ),
            ));
        }
    }
    Ok(())
}
