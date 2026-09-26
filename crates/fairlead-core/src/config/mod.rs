//! The config schema. Every field has a default, so any layer on its own is a
//! valid config, and the same types read TOML and YAML.

mod load;
mod validate;

pub use load::{find_config, load, ConfigError, LoadOptions, Loaded, LOCAL_NAMES, PROJECT_NAMES};
pub use validate::{validate, Problem};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A list that appends to the layers below it, or replaces them when written
/// as `{ replace = [...] }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, expecting = "a list, or { replace = [...] }")]
#[schemars(rename = "List_of_{T}")]
pub enum List<T> {
    Items(Vec<T>),
    Replace(Replace<T>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "Replace_of_{T}")]
pub struct Replace<T> {
    pub replace: Vec<T>,
}

impl<T> List<T> {
    pub fn items(&self) -> &[T] {
        match self {
            List::Items(items) => items,
            List::Replace(r) => &r.replace,
        }
    }
}

impl<T> Default for List<T> {
    fn default() -> Self {
        List::Items(Vec::new())
    }
}

impl<T> From<Vec<T>> for List<T> {
    fn from(items: Vec<T>) -> Self {
        List::Items(items)
    }
}

/// The JSON Schema for `fairlead.toml` and `fairlead.yaml`, for editors.
pub fn json_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Config)).expect("schema serializes")
}

fn strings(items: &[&str]) -> List<String> {
    List::Items(items.iter().map(|s| s.to_string()).collect())
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// The oldest Fairlead version this config needs, such as "0.2".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fairlead: Option<String>,
    pub modules: Modules,
    pub graph: Graph,
    pub tests: Tests,
    pub checks: List<Check>,
    pub plan: Plan,
    pub replay: Replay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Modules {
    /// Where modules come from, in order.
    pub discover: List<Discover>,
    /// Extra modules: each directory matching `pattern` is one, named by `{name}`.
    pub define: List<ModuleDef>,
}

impl Default for Modules {
    fn default() -> Self {
        Modules {
            discover: List::Items(vec![Discover::Workspaces]),
            define: List::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Discover {
    /// One module per package in the workspace manifests.
    Workspaces,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleDef {
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Graph {
    /// "auto" finds the nearest tsconfig to each file; a path uses that one.
    pub tsconfig: String,
    /// Whether `import type` counts as an edge.
    pub type_imports: bool,
    pub unresolved: Unresolved,
    /// Package `exports` conditions, in priority order.
    pub conditions: List<String>,
    /// Keep each file's parse result under `.git/fairlead`, keyed by its
    /// git blob id, so unchanged files aren't parsed again.
    pub cache: bool,
}

impl Default for Graph {
    fn default() -> Self {
        Graph {
            tsconfig: "auto".into(),
            type_imports: true,
            unresolved: Unresolved::Warn,
            conditions: strings(&["import", "node", "default"]),
            cache: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Unresolved {
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Tests {
    #[serde(rename = "match")]
    pub matches: List<String>,
    pub exclude: List<String>,
    /// What a changed file nothing reaches selects. The root and modules
    /// without tests always widen to all.
    pub unreached: Unreached,
    pub runners: List<Runner>,
    pub owners: List<Owner>,
    pub classes: List<ClassRule>,
}

impl Default for Tests {
    fn default() -> Self {
        Tests {
            matches: strings(&["**/*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}"]),
            exclude: strings(&["**/node_modules/**"]),
            unreached: Unreached::Module,
            runners: List::default(),
            owners: List::default(),
            classes: List::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Unreached {
    Module,
    All,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    pub id: String,
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    #[serde(default)]
    pub invoke: Invoke,
    /// Working directory; `{module}` is the module's root path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// An argv array; `{files}` expands to one argument per file.
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Invoke {
    #[default]
    Once,
    PerModule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    /// Test files this rule claims; `{name}` captures one path segment.
    #[serde(rename = "match")]
    pub matches: String,
    /// Paths whose change selects those tests; may use the same `{name}`.
    pub covers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClassRule {
    pub class: TestClass,
    #[serde(rename = "match")]
    pub matches: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TestClass {
    Unit,
    Own,
    Demand,
    Canary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    /// What `{files}` expands to, if the command uses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<CheckFiles>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CheckFiles {
    Changed,
    Matched,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Plan {
    /// A changed path matching any of these selects everything.
    pub run_all: List<String>,
}

impl Default for Plan {
    fn default() -> Self {
        Plan {
            run_all: strings(&[
                "package.json",
                "package-lock.json",
                "pnpm-lock.yaml",
                "pnpm-workspace.yaml",
                "yarn.lock",
                "bun.lock",
                "bun.lockb",
                "tsconfig*.json",
                "turbo.json",
                "nx.json",
                "**/project.json",
                "**/vitest.config.*",
                "**/vitest.workspace.*",
                "**/jest.config.*",
                "**/playwright.config.*",
                ".github/workflows/**",
            ]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Replay {
    pub provider: Provider,
    pub window_days: u32,
    pub min_failures: u32,
    pub failures: List<FailureSource>,
    pub checks: List<CheckStep>,
}

impl Default for Replay {
    fn default() -> Self {
        Replay {
            provider: Provider::Github,
            window_days: 90,
            min_failures: 30,
            failures: List::default(),
            checks: List::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    Github,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailureSource {
    /// The runner whose files these failures name.
    pub runner: String,
    /// A built-in extractor ("vitest", "jest") or "regex" with `pattern`.
    pub extractor: String,
    /// CI job names this source reads, as a regex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// For `extractor = "regex"`: a pattern with a named `file` group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// The CI step name, as a regex.
    pub step: String,
    pub check: String,
}
