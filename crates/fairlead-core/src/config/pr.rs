//! `[pr]`: what a change must meet before review, declared once and checked
//! by `fairlead done`, the pre-push hook and `fairlead pr check` in CI.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::validate::{globs, problem, Problem};
use super::Config;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Pr {
    /// The branch changes target: a pull request against another fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The branch contains the base's head, so it isn't stale or cut from
    /// another branch. Needs `base`.
    pub up_to_date: bool,
    /// A regex the branch name matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// A regex the title matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<Ticket>,
    /// Headings the body has, by their text, such as "Summary".
    pub body_headings: Vec<String>,
    pub labels: Labels,
    /// Files the change must add or change.
    pub files: Vec<RequiredFile>,
}

impl Default for Pr {
    fn default() -> Self {
        Pr {
            base: None,
            up_to_date: true,
            branch: None,
            title: None,
            ticket: None,
            body_headings: Vec::new(),
            labels: Labels::default(),
            files: Vec::new(),
        }
    }
}

/// A ticket key, such as `[A-Z]+-[0-9]+`, and where it appears.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub pattern: String,
    /// The places it must appear in, each of them unless `any`.
    #[serde(rename = "in", default = "title_only")]
    pub places: Vec<Place>,
    /// One of the places is enough.
    #[serde(default)]
    pub any: bool,
}

fn title_only() -> Vec<Place> {
    vec![Place::Title]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Place {
    Branch,
    Title,
    Body,
}

impl Place {
    pub fn name(self) -> &'static str {
        match self {
            Place::Branch => "branch",
            Place::Title => "title",
            Place::Body => "body",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Labels {
    /// Every one of these.
    pub require: Vec<String>,
    /// At least one of these.
    pub require_any: Vec<String>,
    /// None of these.
    pub forbid: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequiredFile {
    /// Globs; the change adds or changes a file matching one of them.
    pub path: Vec<String>,
    /// Only when the change touches one of these globs; any change when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<String>,
    /// A label that excuses the change from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unless_label: Option<String>,
    /// Said beside the failure, such as where the file's format is described.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

fn regex(key: &str, pattern: &str, problems: &mut Vec<Problem>) {
    if let Err(e) = regex::Regex::new(pattern) {
        problems.push(problem(key, e.to_string()));
    }
}

pub(super) fn validate(config: &Config, problems: &mut Vec<Problem>) {
    let Some(pr) = &config.pr else { return };
    for (key, pattern) in [("pr.branch", &pr.branch), ("pr.title", &pr.title)] {
        if let Some(p) = pattern {
            regex(key, p, problems);
        }
    }
    if let Some(t) = &pr.ticket {
        regex("pr.ticket.pattern", &t.pattern, problems);
        if t.places.is_empty() {
            problems.push(problem(
                "pr.ticket.in",
                "names no place: branch, title or body",
            ));
        }
    }
    if pr.base.as_deref().is_some_and(|b| b.trim().is_empty()) {
        problems.push(problem("pr.base", "names no branch"));
    }
    for (i, f) in pr.files.iter().enumerate() {
        if f.path.is_empty() {
            problems.push(problem(
                format!("pr.files[{i}].path"),
                "needs at least one glob",
            ));
        }
        globs(&format!("pr.files[{i}].path"), &f.path, problems);
        globs(&format!("pr.files[{i}].when"), &f.when, problems);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problems(toml_text: &str) -> Vec<String> {
        let config: Config = toml::from_str(toml_text).unwrap();
        let mut out = Vec::new();
        validate(&config, &mut out);
        out.into_iter()
            .map(|p| format!("{}: {}", p.key, p.message))
            .collect()
    }

    #[test]
    fn a_regex_that_doesnt_compile_and_an_empty_glob_list_are_problems() {
        let found = problems(
            "[pr]\nbranch = '(feat'\ntitle = '^[a-z]+: .+'\n[pr.ticket]\npattern = '[A-Z]+-[0-9]+'\nin = []\n[[pr.files]]\npath = []\n",
        );
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found[0].starts_with("pr.branch: "), "{found:?}");
        assert!(found[1].starts_with("pr.ticket.in"), "{found:?}");
        assert!(found[2].starts_with("pr.files[0].path"), "{found:?}");
    }

    #[test]
    fn a_ticket_is_looked_for_in_the_title_unless_it_says_where() {
        let config: Config = toml::from_str("[pr.ticket]\npattern = 'X-[0-9]+'\n").unwrap();
        assert_eq!(
            config.pr.unwrap().ticket.unwrap().places,
            vec![Place::Title]
        );
    }
}
