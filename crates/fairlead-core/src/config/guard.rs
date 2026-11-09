//! `[guard]`: the rules checked as an agent writes, at commit and in CI.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{strings, List};

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
    /// The write hook's own time, in milliseconds; past it the write goes
    /// ahead and the event log says so.
    pub budget_ms: u32,
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
    /// Secret-bearing files a commit may not add, such as keystores and
    /// `.env` files. On unless `false` or `enabled = false`.
    pub paths: PathsSetting,
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
    /// Fetch the base from its remote before comparing numbers with it: at
    /// a push always, at a commit when the last fetch is older than ten
    /// minutes. Offline, the local ref is used with a warning.
    pub fetch: bool,
}

impl Default for Migrations {
    fn default() -> Self {
        Migrations {
            files: List::default(),
            immutable: true,
            base: None,
            unique_prefix: None,
            fetch: false,
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
    /// A regex over the same line that lets a matched command through, since
    /// the regex crate has no lookahead to say "without this flag".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unless: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalRule {
    /// The rule id its findings carry; a preset's name when it's left out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// A tool Fairlead knows how to run and read, in place of `command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<ExternalPreset>,
    /// An argv array; `{files}` expands to the staged files at the commit
    /// stage and to nothing at the check stage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// `check` for a command; `commit` and `check` for a preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stages: Option<Vec<Stage>>,
    #[serde(default)]
    pub ratchet: bool,
    /// Count only the findings the change adds: the command runs on the
    /// base too, and a finding there with the same file and message, line
    /// numbers aside, doesn't count.
    #[serde(default)]
    pub diff: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalPreset {
    /// gitleaks 8.19 or later, for secrets in the content of files.
    Gitleaks,
}

impl ExternalPreset {
    pub fn name(self) -> &'static str {
        match self {
            ExternalPreset::Gitleaks => "gitleaks",
        }
    }
}

impl ExternalRule {
    /// The id findings carry: `id`, else the preset's name.
    pub fn rule_id(&self) -> String {
        match (&self.preset, self.id.trim().is_empty()) {
            (Some(p), true) => p.name().to_string(),
            _ => self.id.clone(),
        }
    }

    pub fn stages(&self) -> Vec<Stage> {
        match (&self.stages, self.preset) {
            (Some(s), _) => s.clone(),
            (None, Some(_)) => vec![Stage::Commit, Stage::Check],
            (None, None) => vec![Stage::Check],
        }
    }
}

impl Default for Guard {
    fn default() -> Self {
        Guard {
            baseline: "fairlead-baseline.json".into(),
            exclude: List::default(),
            findings: Findings::default(),
            on_finding: OnFinding::default(),
            events: Events::default(),
            budget_ms: 40,
            cite: Default::default(),
            size: None,
            comments: None,
            test_names: None,
            citations: None,
            migrations: None,
            commands: List::default(),
            external: List::default(),
            paths: PathsSetting::default(),
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

/// `[guard.paths]` as a table, or `true` or `false` for the built-in lists or none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    untagged,
    expecting = "true, false, or a table such as { deny = [...] }"
)]
pub enum PathsSetting {
    Switch(bool),
    Rules(SecretPaths),
}

impl Default for PathsSetting {
    fn default() -> Self {
        PathsSetting::Rules(SecretPaths::default())
    }
}

impl PathsSetting {
    /// The rules in force, or none when the rule is off.
    pub fn rules(&self) -> Option<SecretPaths> {
        match self {
            PathsSetting::Switch(false) => None,
            PathsSetting::Switch(true) => Some(SecretPaths::default()),
            PathsSetting::Rules(r) if !r.enabled => None,
            PathsSetting::Rules(r) => Some(r.clone()),
        }
    }

    /// Whether the rule is on only because it's on by default: no table
    /// sets its `ratchet`, so tracked matches are warnings.
    pub fn built_in(&self) -> bool {
        self.rules().is_some_and(|r| r.ratchet.is_none())
    }
}

/// The built-in deny list: keystores, keys and certificates, env files and
/// files that are named as credentials.
pub const SECRET_DENY: [&str; 10] = [
    "**/*.jks",
    "**/*.keystore",
    "**/*.p12",
    "**/*.pem",
    "**/*.key",
    "**/.env",
    "**/.env.*",
    "**/key.txt",
    "**/client_secret*",
    "**/*.mobileprovision",
];

/// The built-in allow list: templates of env files, and public keys.
pub const SECRET_ALLOW: [&str; 10] = [
    "**/.env.example",
    "**/.env.sample",
    "**/.env.template",
    "**/.env.test",
    "**/.env.testing",
    "**/.env.ci",
    "**/.env.dist",
    "**/*.example",
    "**/*.pub",
    "**/*.pub.pem",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct SecretPaths {
    pub enabled: bool,
    /// Paths a commit may not add. Appends to the built-in list;
    /// `{ replace = [...] }` replaces it.
    pub deny: List<String>,
    /// Paths that match `deny` and are still fine, such as templates.
    pub allow: List<String>,
    /// Directory names whose matches are fixtures: warnings, never failures.
    pub fixtures: List<String>,
    /// Unset, tracked matches are warnings at the check stage. `true` holds
    /// them at the baseline, `false` fails on every one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratchet: Option<bool>,
}

impl Default for SecretPaths {
    fn default() -> Self {
        SecretPaths {
            enabled: true,
            deny: strings(&SECRET_DENY),
            allow: strings(&SECRET_ALLOW),
            fixtures: strings(&["test", "tests", "fixtures", "testdata", "__tests__", "spec"]),
            ratchet: None,
        }
    }
}
