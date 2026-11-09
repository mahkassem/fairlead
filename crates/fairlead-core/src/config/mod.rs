//! The config schema. Every field has a default, so any layer on its own is a
//! valid config, and the same types read TOML and YAML.

mod agents;
mod guard;
mod guard_validate;
mod knowledge;
mod load;
mod needs;
pub mod packs;
mod planning;
mod pr;
mod quarantine;
mod stages;
mod tracker;
mod validate;
pub mod workspace;

pub use agents::{Agents, AgentsWrite};
pub use guard::*;
pub use guard_validate::GUARD_RULES;
pub use knowledge::{Learn, Memory, SkillRoute, Skills};
pub use load::{
    find_config, load, load_file, read_layer, ConfigError, LoadOptions, Loaded, ENV_NAME,
    LOCAL_NAMES, PROJECT_NAMES,
};
pub use needs::{env_names, Need};
pub use planning::{Globs, LockfileMode, Plan, RunAll, RunAllKinds};
pub use pr::{Labels, Place, Pr, RequiredFile, Ticket};
pub use quarantine::{Condition, Os, PlatformQuarantine};
pub use stages::{CiStage, Phase, Stages};
pub use tracker::{Tracker, TrackerKind};
pub use validate::{is_date, plan_globs, validate, Problem};

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

    pub fn is_empty(&self) -> bool {
        self.items().is_empty()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    /// The planner, the done gate and the guard.
    #[default]
    Full,
    /// The guard and the `[pr]` rules only.
    Guard,
}

impl Profile {
    pub fn is_full(&self) -> bool {
        *self == Profile::Full
    }
}

fn strings<T: for<'a> From<&'a str>>(items: &[&str]) -> List<T> {
    List::Items(items.iter().map(|s| T::from(s)).collect())
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// The oldest Fairlead version this config needs, such as "0.2".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fairlead: Option<String>,
    /// Where the project is in its life: before a live deployment, whole
    /// suites wait for the full stage unless a step says otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    /// `guard`: only the guard and the `[pr]` rules, with no test planner,
    /// for a stack the planner doesn't read. Left out of the digest.
    #[serde(skip_serializing_if = "Profile::is_full")]
    pub profile: Profile,
    /// Framework packs layered under this config: a built-in pack's name,
    /// such as "laravel", or a pack file's path from the project root. Left
    /// out of the digest when empty; a pack's rules count through the merge.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    pub modules: Modules,
    pub graph: Graph,
    pub tests: Tests,
    pub checks: List<Check>,
    /// Tests and checks that fail on one platform whatever the change, held
    /// there with their evidence. Left out of the digest when empty, so
    /// configs without them keep their plan ids.
    #[serde(skip_serializing_if = "List::is_empty")]
    pub quarantine: List<PlatformQuarantine>,
    /// What some tests need from the machine, and how to check for it. Never
    /// changes a plan, so it's left out of the digest.
    #[serde(skip_serializing_if = "List::is_empty")]
    pub needs: List<Need>,
    pub plan: Plan,
    pub replay: Replay,
    pub guard: Guard,
    pub hooks: Hooks,
    pub done: Done,
    pub brief: Brief,
    pub ci: Ci,
    pub memory: Memory,
    pub skills: Skills,
    pub agents: Agents,
    /// When each runner and check runs in CI; left out of the digest, and
    /// absent unless a layer sets it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stages: Option<Stages>,
    /// Where a change's task lives. Left out of the digest unless set.
    #[serde(skip_serializing_if = "Tracker::is_unset")]
    pub tracker: Tracker,
    /// Only in a workspace file, in a folder holding several repositories;
    /// a repository's own file can't set it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Workspace>,
    /// What a change meets before review; left out of the digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<Pr>,
}

/// The repositories of a workspace folder.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Workspace {
    /// The repositories, by path from the workspace folder. Empty means
    /// every git repository one level down.
    pub repos: Vec<WorkspaceRepo>,
    /// Dependencies between repositories that no import shows, such as a
    /// contract one repository owns and another consumes.
    pub edges: Vec<WorkspaceEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEdge {
    /// Files, from the workspace folder, that depend on what `to` names.
    pub from: String,
    /// Files in another repository, from the workspace folder.
    pub to: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRepo {
    /// The repository's folder, from the workspace folder.
    pub path: String,
    /// What `--repo` calls it; defaults to the folder's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The branch or commit its changes are measured from; defaults to its
    /// remote's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
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
    /// Whether a named import is followed through `export … from` to the
    /// file that declares the name. Left out of the digest when on.
    #[serde(skip_serializing_if = "is_true")]
    pub reexports: bool,
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
    /// Commands that print a graph for the files they claim, for languages
    /// and build tools the built-in scanner doesn't read. Left out of the
    /// digest when empty, so configs without them keep their plan ids.
    #[serde(skip_serializing_if = "List::is_empty")]
    pub providers: List<GraphProvider>,
    /// A coverage map whose edges join the static graph. Off by default,
    /// and left out of the digest when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
}

fn is_true(b: &bool) -> bool {
    *b
}

/// Where the coverage map is, and how old it may get before the plan warns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    /// The map, relative to the project root, as `fairlead coverage import` writes it.
    pub map: String,
    #[serde(default = "default_max_age_days")]
    pub max_age_days: u32,
}

fn default_max_age_days() -> u32 {
    14
}

/// An external graph provider: `command` runs at the project root, reads the
/// files it claims on stdin, one per line, and prints `{"version": 1,
/// "edges": [{"from": ..., "to": ...}]}` with repo-relative paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GraphProvider {
    pub id: String,
    pub command: Vec<String>,
    /// The files it claims; the built-in scanner leaves them alone.
    pub files: Vec<String>,
    /// How long it may run before it counts as failed.
    #[serde(default = "provider_timeout")]
    pub timeout_seconds: u64,
}

fn provider_timeout() -> u64 {
    120
}

/// Each file matching `from` depends on every file its `to` globs match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgeRule {
    /// The dependent files; `{name}` stands for one path segment.
    pub from: String,
    /// The files they depend on, with the same `{name}`s as `from`.
    pub to: Vec<String>,
    /// A regex searched in each `from` file's text: the rule links only the
    /// files it matches in, and its capture, if any, fills `{1}` in `to`
    /// (`{1|path}` with its dots as slashes). Left out of the digest when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub find: Option<String>,
}

/// The placeholders `{1}` and `{1|path}` become in a `to` glob, since a
/// capture's name can't start with a digit or hold `|`.
pub const FOUND: &str = "found";
pub const FOUND_PATH: &str = "found_path";

impl EdgeRule {
    /// The `to` globs as the matcher reads them.
    pub fn targets(&self) -> Vec<String> {
        self.to
            .iter()
            .map(|t| {
                t.replace("{1|path}", &format!("{{{FOUND_PATH}}}"))
                    .replace("{1}", &format!("{{{FOUND}}}"))
            })
            .collect()
    }
}

impl Default for Graph {
    fn default() -> Self {
        Graph {
            tsconfig: "auto".into(),
            type_imports: true,
            reexports: true,
            unresolved: Unresolved::Warn,
            conditions: strings(&["import", "node", "default"]),
            cache: true,
            edges: List::default(),
            barrier: List::default(),
            providers: List::default(),
            coverage: None,
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
    /// The argv when everything runs, for a tool that left without files
    /// finds more tests than `match` claims; `command` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_command: Option<Vec<String>>,
    /// An argv fragment holding `{file}`, added once per held test file, so
    /// a run of everything can leave out what runs alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_arg: Option<Vec<String>>,
    /// The earliest CI stage this runner runs at; `ready` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<CiStage>,
    /// The earliest stage its whole suite runs at; earlier, it runs only
    /// the tests a change reaches. Unset, the project's `phase` decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_from: Option<CiStage>,
    /// Quick enough that `done` runs it first, or adds it, when CI history
    /// shows it going red; without it, its median time decides.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cheap: bool,
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
    /// The earliest CI stage this check runs at; `draft` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<CiStage>,
    /// The earliest stage it runs over every file it matches; earlier, only
    /// on changed files. Unset, the project's `phase` decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_from: Option<CiStage>,
    /// Quick enough that `done` runs it first, or adds it, when CI history
    /// shows it going red; without it, its median time decides.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cheap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CheckFiles {
    Changed,
    Matched,
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
    /// A built-in extractor ("vitest", "jest", "bun", "phpunit", "pest", "go", "pytest",
    /// "maven", "gradle") or "regex" with `pattern`.
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

/// What `fairlead done` runs before a change counts as finished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Done {
    /// The test files the change's plan selects, or none.
    pub tests: DoneTests,
    /// The `[[checks]]` the change's plan selects, or none.
    pub checks: DoneTests,
    /// `[[checks]]` ids that run for every change, planned or not.
    pub always: List<String>,
    /// `fairlead guard check` over the whole tree, as CI runs it.
    pub guard: bool,
    /// What the Claude Code Stop hook does when the tree an agent leaves
    /// hasn't passed the gate.
    pub on_stop: OnStop,
    /// A runner or check whose median time is at most this many seconds
    /// counts as cheap, for the CI history `done` reads.
    pub cheap_seconds: u32,
    /// Install the pre-push hook, `fairlead done --pre-push`, even with no
    /// `[pr]` rules or migration fetch asking for it.
    pub pre_push: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OnStop {
    /// No Stop hook.
    Off,
    /// Send the agent back once with what's left; its next stop goes through.
    #[default]
    Ask,
    /// Send it back until the gate passes; Claude Code's own cap ends a loop.
    Require,
}

impl Default for Done {
    fn default() -> Self {
        Done {
            tests: DoneTests::Planned,
            checks: DoneTests::Planned,
            always: List::default(),
            guard: true,
            on_stop: OnStop::Ask,
            cheap_seconds: 60,
            pre_push: false,
        }
    }
}

/// What `fairlead ci` does beyond running the plan.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Ci {
    /// Also keep one pull request comment up to date with the report, which
    /// needs `pull-requests: write`; the step summary always gets it.
    pub comment: bool,
    /// What a failing test the merge's plan left out does, once `ci run --judge` finds it.
    pub escapes: Escapes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Escapes {
    /// An annotation and a line in the output.
    #[default]
    Report,
    /// Also a non-zero exit from `ci report`, so the escape itself can be a required check.
    Fail,
    /// Also a GitHub issue per escape, labelled `fairlead-escape`: at most
    /// five new ones a run, and a repeat is a note on the open one.
    File,
}

/// What `fairlead brief` keeps, and whether an agent that edits without one is told.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Brief {
    /// Whether a second brief in the same session adds its paths to the
    /// first, or stands alone.
    pub per: BriefPer,
    /// A one-line note, once per session, after the first edit made with no brief.
    pub nudge: bool,
    /// Whether `hooks install` adds the `SessionStart` hook that runs `fairlead resume`.
    pub resume: bool,
}

impl Default for Brief {
    fn default() -> Self {
        Brief {
            per: BriefPer::Session,
            nudge: true,
            resume: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BriefPer {
    /// One brief per session: a later brief adds its paths, so a change that
    /// grows is still one brief.
    #[default]
    Session,
    /// Each brief stands alone; the newest is the one that counts.
    Call,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DoneTests {
    #[default]
    Planned,
    None,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Hooks {
    /// Where `fairlead hooks install` puts the Claude Code hook.
    pub claude: HooksTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HooksTarget {
    /// `.claude/settings.json`, committed, so everyone who clones is guarded.
    #[default]
    Shared,
    /// `.claude/settings.local.json`, for one person, ignored by git.
    Local,
}
