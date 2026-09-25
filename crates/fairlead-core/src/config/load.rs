//! Finding, reading and merging the config layers. Layers merge as plain
//! values: maps merge by key, lists append unless written as
//! `{ replace = [...] }`, and anything else is overridden. Each layer is also
//! checked on its own, so an error names the file it came from.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{validate, Config, Problem};

pub const PROJECT_NAMES: [&str; 3] = ["fairlead.toml", "fairlead.yaml", "fairlead.yml"];
pub const LOCAL_NAMES: [&str; 3] = [
    "fairlead.local.toml",
    "fairlead.local.yaml",
    "fairlead.local.yml",
];
const ENV_PREFIX: &str = "FAIRLEAD_";
const ENV_SEPARATOR: &str = "__";

#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// `key=value` pairs from `--set`, applied last.
    pub sets: Vec<String>,
    /// Environment variables to read `FAIRLEAD_*` layers from.
    pub env: Vec<(String, String)>,
    /// In CI the local file is ignored.
    pub ci: bool,
}

impl LoadOptions {
    pub fn from_process(sets: Vec<String>) -> Self {
        let env: Vec<(String, String)> = std::env::vars().collect();
        let ci = env
            .iter()
            .any(|(k, v)| k == "CI" && !matches!(v.as_str(), "" | "0" | "false"));
        LoadOptions { sets, env, ci }
    }
}

#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    /// The layer that set each value, by dotted key.
    pub origins: BTreeMap<String, String>,
    /// The merged config as a plain value, for display.
    pub value: Value,
    /// Files read, project first.
    pub files: Vec<PathBuf>,
    /// The directory holding the project file, or the start directory if none.
    pub root: PathBuf,
    /// Semantic problems that don't stop loading.
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    /// The file or layer the error came from.
    pub source: String,
    /// The dotted key, when the error is about one.
    pub key: Option<String>,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.key {
            Some(key) if !key.is_empty() && key != "." => {
                write!(f, "{}: {}: {}", self.source, key, self.message)
            }
            _ => write!(f, "{}: {}", self.source, self.message),
        }
    }
}

impl std::error::Error for ConfigError {}

fn error(source: &str, key: Option<String>, message: impl Into<String>) -> ConfigError {
    ConfigError {
        source: source.to_string(),
        key,
        message: message.into(),
    }
}

/// The nearest directory holding a project file, walking up from `start`
/// and stopping at the repository root (the first directory with `.git`),
/// so a config outside the repository is never read. Two project files in
/// one directory is an error.
pub fn find_config(start: &Path) -> Result<Option<PathBuf>, ConfigError> {
    for dir in start.ancestors() {
        let found: Vec<PathBuf> = PROJECT_NAMES
            .iter()
            .map(|n| dir.join(n))
            .filter(|p| p.is_file())
            .collect();
        match found.len() {
            0 if dir.join(".git").exists() => return Ok(None),
            0 => continue,
            1 => return Ok(found.into_iter().next()),
            _ => {
                let names: Vec<String> = found.iter().map(|p| file_name(p)).collect();
                return Err(error(
                    &dir.display().to_string(),
                    None,
                    format!("more than one config file: {}", names.join(", ")),
                ));
            }
        }
    }
    Ok(None)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn load(start: &Path, opts: &LoadOptions) -> Result<Loaded, ConfigError> {
    let project = find_config(start)?;
    let root = project
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| start.to_path_buf());
    let mut layers: Vec<(String, Value)> = Vec::new();
    let mut files = Vec::new();
    if let Some(path) = &project {
        layers.push((file_name(path), read_file(path)?));
        files.push(path.clone());
    }
    if !opts.ci {
        if let Some(path) = local_file(&root)? {
            layers.push((file_name(&path), read_file(&path)?));
            files.push(path);
        }
    }
    let mut env: Vec<&(String, String)> = opts.env.iter().collect();
    env.sort();
    for (name, raw) in env {
        if let Some(key) = env_key(name) {
            layers.push((format!("env {name}"), nested(&key, parse_scalar(raw))));
        }
    }
    for set in &opts.sets {
        let (key, raw) = set
            .split_once('=')
            .ok_or_else(|| error("--set", None, format!("expected key=value, got `{set}`")))?;
        layers.push((
            "--set".to_string(),
            nested(key.trim(), parse_scalar(raw.trim())),
        ));
    }
    for (label, value) in &layers {
        typed(label, value.clone())?;
    }
    let mut merged = serde_json::to_value(Config::default()).expect("defaults serialize");
    let mut origins = BTreeMap::new();
    record(&merged, "", "default", &mut origins);
    for (label, value) in layers {
        merge(&mut merged, value, &label, "", &mut origins);
    }
    let config = typed("merged config", merged.clone())?;
    let problems = validate(&config);
    Ok(Loaded {
        config,
        origins,
        value: merged,
        files,
        root,
        problems,
    })
}

fn local_file(root: &Path) -> Result<Option<PathBuf>, ConfigError> {
    let found: Vec<PathBuf> = LOCAL_NAMES
        .iter()
        .map(|n| root.join(n))
        .filter(|p| p.is_file())
        .collect();
    if found.len() > 1 {
        let names: Vec<String> = found.iter().map(|p| file_name(p)).collect();
        return Err(error(
            &root.display().to_string(),
            None,
            format!("more than one local config file: {}", names.join(", ")),
        ));
    }
    Ok(found.into_iter().next())
}

fn read_file(path: &Path) -> Result<Value, ConfigError> {
    let label = file_name(path);
    let text = std::fs::read_to_string(path).map_err(|e| error(&label, None, e.to_string()))?;
    let value: Value = if label.ends_with(".toml") {
        toml::from_str(&text).map_err(|e| error(&label, None, e.to_string().trim().to_string()))?
    } else {
        serde_saphyr::from_str(&text)
            .map_err(|e| error(&label, None, e.to_string().trim().to_string()))?
    };
    Ok(match value {
        Value::Null => Value::Object(Map::new()),
        other => other,
    })
}

fn typed(label: &str, value: Value) -> Result<Config, ConfigError> {
    serde_path_to_error::deserialize(value).map_err(|e| {
        let key = e.path().to_string();
        error(label, Some(key), e.into_inner().to_string())
    })
}

/// `FAIRLEAD_TESTS__UNREACHED` is `tests.unreached`. A name without the
/// separator isn't config, which leaves room for other variables.
fn env_key(name: &str) -> Option<String> {
    let rest = name.strip_prefix(ENV_PREFIX)?;
    if !rest.contains(ENV_SEPARATOR) {
        return None;
    }
    Some(
        rest.split(ENV_SEPARATOR)
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// A TOML boolean, number or array when it parses as one (`true`, `3`,
/// `["a"]`), else a string; a date stays a string.
fn parse_scalar(raw: &str) -> Value {
    toml::from_str::<toml::Table>(&format!("v = {raw}"))
        .ok()
        .and_then(|t| t.get("v").cloned())
        .filter(|v| !matches!(v, toml::Value::Datetime(_) | toml::Value::Table(_)))
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_else(|| Value::String(raw.to_string()))
}

fn nested(key: &str, value: Value) -> Value {
    key.rsplit('.').fold(value, |inner, part| {
        let mut map = Map::new();
        map.insert(part.to_string(), inner);
        Value::Object(map)
    })
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

fn replacement(value: &Value) -> Option<&Vec<Value>> {
    match value {
        Value::Object(map) if map.len() == 1 => map.get("replace").and_then(Value::as_array),
        _ => None,
    }
}

fn merge(
    base: &mut Value,
    over: Value,
    label: &str,
    path: &str,
    origins: &mut BTreeMap<String, String>,
) {
    if let Some(items) = replacement(&over) {
        *base = Value::Array(items.clone());
        origins.retain(|k, _| k != path && !k.starts_with(&format!("{path}.")));
        origins.insert(path.to_string(), label.to_string());
        return;
    }
    match (base, over) {
        (Value::Object(base_map), Value::Object(over_map)) => {
            for (key, value) in over_map {
                let child = join(path, &key);
                let slot = base_map.entry(key).or_insert(Value::Null);
                merge(slot, value, label, &child, origins);
            }
        }
        (Value::Array(base_items), Value::Array(over_items)) => {
            if !over_items.is_empty() {
                base_items.extend(over_items);
                let before = origins.get(path).cloned();
                let joined = match before {
                    Some(b) if b != label => format!("{b} + {label}"),
                    _ => label.to_string(),
                };
                origins.insert(path.to_string(), joined);
            }
        }
        (slot, value) => {
            origins.retain(|k, _| k != path && !k.starts_with(&format!("{path}.")));
            record(&value, path, label, origins);
            *slot = value;
        }
    }
}

fn record(value: &Value, path: &str, label: &str, origins: &mut BTreeMap<String, String>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                record(child, &join(path, key), label, origins);
            }
        }
        _ => {
            origins.insert(path.to_string(), label.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn env_names_map_to_dotted_keys_and_others_are_ignored() {
        assert_eq!(
            env_key("FAIRLEAD_TESTS__UNREACHED").as_deref(),
            Some("tests.unreached")
        );
        assert_eq!(
            env_key("FAIRLEAD_PLAN__RUN_ALL").as_deref(),
            Some("plan.run_all")
        );
        assert_eq!(env_key("FAIRLEAD_BYPASS"), None);
        assert_eq!(env_key("OTHER__THING"), None);
    }

    #[test]
    fn scalars_parse_as_toml_literals_or_fall_back_to_strings() {
        assert_eq!(parse_scalar("true"), json!(true));
        assert_eq!(parse_scalar("30"), json!(30));
        assert_eq!(parse_scalar("[\"a\", \"b\"]"), json!(["a", "b"]));
        assert_eq!(parse_scalar("all"), json!("all"));
        assert_eq!(parse_scalar("2024-01-01"), json!("2024-01-01"));
    }

    #[test]
    fn lists_append_and_replace_replaces() {
        let mut origins = BTreeMap::new();
        let mut base = json!({ "plan": { "run_all": ["a"] } });
        record(&base, "", "default", &mut origins);
        merge(
            &mut base,
            json!({ "plan": { "run_all": ["b"] } }),
            "one",
            "",
            &mut origins,
        );
        assert_eq!(base, json!({ "plan": { "run_all": ["a", "b"] } }));
        assert_eq!(origins["plan.run_all"], "default + one");
        merge(
            &mut base,
            json!({ "plan": { "run_all": { "replace": ["c"] } } }),
            "two",
            "",
            &mut origins,
        );
        assert_eq!(base, json!({ "plan": { "run_all": ["c"] } }));
        assert_eq!(origins["plan.run_all"], "two");
    }

    #[test]
    fn scalars_override_and_record_their_layer() {
        let mut origins = BTreeMap::new();
        let mut base = json!({ "tests": { "unreached": "module" } });
        record(&base, "", "default", &mut origins);
        merge(
            &mut base,
            json!({ "tests": { "unreached": "all" } }),
            "--set",
            "",
            &mut origins,
        );
        assert_eq!(base["tests"]["unreached"], "all");
        assert_eq!(origins["tests.unreached"], "--set");
    }
}
