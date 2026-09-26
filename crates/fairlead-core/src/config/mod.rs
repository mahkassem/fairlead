//! The config schema. Every field has a default, so any layer on its own is a
//! valid config, and the same types read TOML and YAML.

mod load;
mod validate;

pub use load::{
    find_config, load, load_file, ConfigError, LoadOptions, Loaded, ENV_NAME, LOCAL_NAMES,
    PROJECT_NAMES,
};
pub use validate::{validate, Problem, GUARD_RULES};

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
    pub guard: Guard,
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
    /// Dependencies the imports don't show, followed like imports.
    pub edges: List<EdgeRule>,
    /// Files the walk reaches but never goes past to their importers.
    pub barrier: List<String>,
}

/// Each file matching `from` depends on every file its `to` globs match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgeRule {
    /// The dependent files; `{name}` stands for one path segment.
    pub from: String,
    /// The files they depend on, with the same `{name}`s as `from`.
    pub to: Vec<String>,
}

impl Default for Graph {
    fn default() -> Self {
        Graph {
            tsconfig: "auto".into(),
            type_imports: true,
            unresolved: Unresolved::Warn,
            conditions: strings(&["import", "node", "default"]),
            cache: true,
            edges: List::default(),
            barrier: List::default(),
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
    /// Test files under `match` that this runner leaves to another.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
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
    /// A covered path that matches `plan.run_all` selects this rule's tests
    /// instead of every test, such as a fixture project's runner config.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overrides_run_all: bool,
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
    /// A changed path matching these selects nothing by itself; files that
    /// reference it are still reached through their edges.
    pub ignore: List<String>,
    /// `all`: a changed lockfile selects everything. `scope`: a changed pnpm
    /// lockfile selects only the workspace packages whose resolved
    /// dependencies changed; opt-in while the benchmarks gather evidence.
    pub lockfile: LockfileMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LockfileMode {
    Scope,
    #[default]
    All,
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
            ignore: strings(&[
                "*.md",
                ".changeset/**",
                "docs/**",
                "**/README.md",
                "**/CHANGELOG.md",
                "LICENSE*",
            ]),
            lockfile: LockfileMode::default(),
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
    /// CI job names (regexes) whose failures replay leaves out on purpose.
    pub ignore: List<String>,
    /// Step names (regexes): a job that failed only in such steps, such as
    /// an install, failed before any test and is left out.
    pub ignore_steps: List<String>,
    /// Tests declared flaky in one job, with evidence and an expiry.
    pub quarantine: List<Quarantine>,
}

impl Default for Replay {
    fn default() -> Self {
        Replay {
            provider: Provider::Github,
            window_days: 90,
            min_failures: 30,
            failures: List::default(),
            checks: List::default(),
            ignore: List::default(),
            ignore_steps: List::default(),
            quarantine: List::default(),
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
    /// A built-in extractor ("vitest", "jest", "bun") or "regex" with `pattern`.
    pub extractor: String,
    /// CI job names this source reads, as a regex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// For `extractor = "regex"`: a pattern with a named `file` group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

/// A test file declared flaky in the jobs `job` names. Replay still plans
/// and judges its failures, reports what they would have been, and applies
/// the entry only while the dataset bears it out and `until` hasn't passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Quarantine {
    /// The test file, exactly as the repository names it.
    pub path: String,
    /// The CI job names it fails in, as a regex.
    pub job: String,
    /// The evidence, in a sentence.
    pub reason: String,
    /// The last day the entry applies, `YYYY-MM-DD`.
    pub until: String,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Guard {
    /// Ratcheted counts per file and rule, relative to the project root.
    pub baseline: String,
    /// Tracked files no rule reads.
    pub exclude: List<String>,
    /// Which findings count at a write or a commit: only those the change
    /// adds, or every finding in a file it touches.
    pub findings: Findings,
    /// Whether those findings stop the write or commit, or are only shown.
    pub on_finding: OnFinding,
    /// Where hook and commit decisions are recorded.
    pub events: Events,
    /// A note appended to each finding of a rule, by rule id, such as the
    /// section of a style guide it enforces.
    pub cite: std::collections::BTreeMap<String, String>,
    /// File and function length, ratcheted unless `ratchet = false`. Off until set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<SizeRules>,
    /// Comment rules, zero-tolerance unless `ratchet = true`. Off until set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<CommentRules>,
    /// Test file names and titles. Off until set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_names: Option<TestNames>,
    /// Pointers in comments that must name a heading in a file. Off until set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<Citations>,
    /// Migrations that may not change once they exist. Off until set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migrations: Option<Migrations>,
    /// Shell commands an agent may not run, checked at the write stage.
    pub commands: List<CommandRule>,
    /// Commands that print findings as `file:line message`.
    pub external: List<ExternalRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct TestNames {
    pub files: List<String>,
    pub exclude: List<String>,
    /// A regex the file's name must match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// A regex no test title may match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub titles_without: Option<String>,
    /// The calls whose first argument is a title.
    pub title_calls: List<String>,
}

impl Default for TestNames {
    fn default() -> Self {
        TestNames {
            files: List::default(),
            exclude: List::default(),
            file: None,
            titles_without: None,
            title_calls: strings(&["describe", "test", "it"]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Citations {
    /// The files whose comments are read.
    pub files: List<String>,
    pub exclude: List<String>,
    /// A regex for a pointer, with the cited name in a group called `code`.
    pub pattern: String,
    /// The Markdown file whose headings the pointers must name.
    pub headings_in: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Migrations {
    pub files: List<String>,
    /// A migration that exists at the base may not change, move or go.
    pub immutable: bool,
    /// What "exists" is measured against: the merge base with this ref.
    /// Without it the commit stage uses HEAD and the check stage needs `--base`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Each file's leading number is unique, apart from the listed groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_prefix: Option<UniquePrefix>,
}

impl Default for Migrations {
    fn default() -> Self {
        Migrations {
            files: List::default(),
            immutable: true,
            base: None,
            unique_prefix: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct UniquePrefix {
    /// File names allowed to share a number, one list per number.
    pub allow: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandRule {
    /// A regex over the whole command line.
    #[serde(rename = "match")]
    pub matches: String,
    /// Why, shown to the agent.
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Write,
    Commit,
    Check,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalRule {
    /// The rule id its findings carry.
    pub id: String,
    /// An argv array; `{files}` expands to the staged files at the commit
    /// stage and to nothing at the check stage.
    pub command: Vec<String>,
    #[serde(default = "check_stage")]
    pub stages: Vec<Stage>,
    #[serde(default)]
    pub ratchet: bool,
}

fn check_stage() -> Vec<Stage> {
    vec![Stage::Check]
}

impl Default for Guard {
    fn default() -> Self {
        Guard {
            baseline: "fairlead-baseline.json".into(),
            exclude: List::default(),
            findings: Findings::default(),
            on_finding: OnFinding::default(),
            events: Events::default(),
            cite: Default::default(),
            size: None,
            comments: None,
            test_names: None,
            citations: None,
            migrations: None,
            commands: List::default(),
            external: List::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Findings {
    /// Only findings the change adds, so old debt doesn't block a fix.
    #[default]
    Added,
    /// Every finding in a file the change touches.
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OnFinding {
    /// Stop it, with the findings as the reason.
    #[default]
    Deny,
    /// Let it through and show the findings; the check stage still fails.
    Warn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Events {
    /// `.git/fairlead/events.jsonl`, never committed.
    #[default]
    Local,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct SizeRules {
    /// The files these rules read.
    pub files: List<String>,
    pub exclude: List<String>,
    /// A file over this many lines is a finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_lines: Option<u32>,
    /// A function over this many of its own lines, not counting the
    /// functions nested in it, is a finding. JavaScript and TypeScript only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_lines: Option<u32>,
    /// Calls whose callback isn't measured itself, only what it nests.
    pub test_hooks: List<String>,
    /// Counted against the baseline rather than failing on any finding.
    pub ratchet: bool,
}

impl Default for SizeRules {
    fn default() -> Self {
        SizeRules {
            files: List::default(),
            exclude: List::default(),
            file_lines: None,
            function_lines: None,
            test_hooks: strings(&[
                "describe",
                "test",
                "it",
                "beforeAll",
                "afterAll",
                "beforeEach",
                "afterEach",
            ]),
            ratchet: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CommentRules {
    /// The files these rules read. The comment syntax comes from the
    /// extension: `//` and `/* */` for JavaScript and TypeScript, `--` for
    /// SQL, `#` for YAML, TOML and shell.
    pub files: List<String>,
    pub exclude: List<String>,
    /// Test files, which take the `test` limits.
    pub tests: List<String>,
    /// Migrations, which take the `migration` block limit and no density limit.
    pub migrations: List<String>,
    /// The longest comment block, in lines, by where it sits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_length: Option<BlockLength>,
    /// The largest share of comment lines among comment and code lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<Density>,
    /// History a comment shouldn't carry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<History>,
    /// Ticket or item references a comment shouldn't carry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_codes: Option<ItemCodes>,
    /// Regexes for a comment that addresses its next editor.
    pub agent_phrases: List<String>,
    /// Each continuation line of a multi-line `/* */` starts with `*`.
    pub block_marker: bool,
    pub ratchet: bool,
}

/// A block's context is the first that applies: migration, inline (indented
/// after a code line), file header (starting on line 1), test, source. A
/// context without a limit falls through to the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlockLength {
    pub source: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Density {
    pub source: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct History {
    /// A date from 2000 to 2099 written `YYYY-MM-DD`.
    pub dates: bool,
    /// Names, matched as whole words and case.
    pub names: Vec<String>,
    /// Phrases, matched as whole words in any case.
    pub phrases: Vec<String>,
    /// A number with `px`, `ms`, `s`, `KB` or `MB` in a comment that says "measured".
    pub measured: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemCodes {
    /// A regex for one reference.
    pub pattern: String,
    /// Allow references as a pointer: `(CODE)` or `(CODE, CODE)` followed by
    /// a full stop or the end of the comment.
    #[serde(default)]
    pub pointer: bool,
}
