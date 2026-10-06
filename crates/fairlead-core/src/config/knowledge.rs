//! The knowledge tables: lessons (`[memory]`) and skill routing (`[skills]`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{strings, List};

/// Lessons: one small file each, offered for a change the way tests are picked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Memory {
    /// Where the lesson files live, from the repository root.
    pub dir: String,
    /// Days from `added` to the `review_by` a new lesson gets.
    pub review_days: u32,
    /// The longest body a lesson may have; anything longer is a doc to link.
    pub max_lines: usize,
    /// Lessons a brief lists before "N more".
    pub cap: usize,
    /// What `fairlead learn` does with the lesson it makes.
    pub learn: Learn,
}

impl Default for Memory {
    fn default() -> Self {
        Memory {
            dir: ".fairlead/lessons".into(),
            review_days: 90,
            max_lines: 12,
            cap: 5,
            learn: Learn::Write,
        }
    }
}

/// Skills in the open SKILL.md format, routed to a change by scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Skills {
    /// Hops along what a changed file imports, for lessons and skills alike.
    pub imports: usize,
    /// Hops along the files that import a changed file; off by default.
    pub importers: usize,
    /// Skills a brief lists before "N more".
    pub cap: usize,
    /// The agents `skills sync` writes skills for.
    pub targets: List<String>,
    /// Which code each skill applies to.
    pub routes: List<SkillRoute>,
}

impl Default for Skills {
    fn default() -> Self {
        Skills {
            imports: 1,
            importers: 0,
            cap: 8,
            targets: strings(&["claude", "agents", "cursor"]),
            routes: List::default(),
        }
    }
}

/// One skill and the scope it applies to: `paths`, `modules` or `always`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillRoute {
    /// The skill's SKILL.md, from the repository root.
    pub skill: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub always: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Learn {
    /// Write the file to the working tree, for a person to review in the pull request.
    #[default]
    Write,
    /// Print the file instead, for a person to save.
    Ask,
}
