//! `[[guard.commands]]`: shell commands an agent may not run, each with the
//! reason it is told. Read by the write stage's Bash hook.

use fairlead_core::config::CommandRule;
use regex::Regex;

struct Rule {
    matches: Regex,
    unless: Option<Regex>,
    reason: String,
}

#[derive(Default)]
pub struct Commands {
    rules: Vec<Rule>,
}

impl Commands {
    pub fn new(rules: &[CommandRule]) -> Result<Commands, String> {
        let regex = |r: &str| Regex::new(r).map_err(|e| e.to_string());
        let rules = rules
            .iter()
            .map(|r| {
                Ok(Rule {
                    matches: regex(&r.matches)?,
                    unless: r.unless.as_deref().map(regex).transpose()?,
                    reason: r.reason.clone(),
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Commands { rules })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The reason the first rule that denies the command gives, if any does:
    /// its `match` matches the line and its `unless`, when set, doesn't.
    pub fn denied(&self, command: &str) -> Option<&str> {
        self.rules
            .iter()
            .find(|r| {
                r.matches.is_match(command)
                    && !r.unless.as_ref().is_some_and(|u| u.is_match(command))
            })
            .map(|r| r.reason.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(m: &str, unless: Option<&str>, r: &str) -> CommandRule {
        CommandRule {
            matches: m.into(),
            unless: unless.map(String::from),
            reason: r.into(),
        }
    }

    #[test]
    fn the_first_matching_rule_gives_its_reason() {
        let c = Commands::new(&[
            rule(
                r"(^|\s)git push --force(\s|$)",
                None,
                "Open a pull request instead.",
            ),
            rule(r"rm -rf /", None, "Never."),
        ])
        .unwrap();
        assert_eq!(
            c.denied("cd x && git push --force origin main"),
            Some("Open a pull request instead.")
        );
        assert_eq!(c.denied("git push --forced"), None);
        assert_eq!(c.denied("ls"), None);
    }

    #[test]
    fn a_matched_command_that_unless_also_matches_goes_through() {
        let c = Commands::new(&[
            rule(
                r"\bdeploy\b",
                Some(r"(^|\s)--dry-run(\s|$)"),
                "Dry run first.",
            ),
            rule(r"\bdeploy\b", None, "Never deploy."),
        ])
        .unwrap();
        assert_eq!(
            c.denied("deploy --dry-run"),
            Some("Never deploy."),
            "an `unless` lets a command past its own rule only"
        );
        let c = Commands::new(&[rule(
            r"\bdeploy\b",
            Some(r"(^|\s)--dry-run(\s|$)"),
            "Dry run first.",
        )])
        .unwrap();
        assert_eq!(c.denied("deploy --dry-run"), None);
        assert_eq!(c.denied("deploy --dry-runs"), Some("Dry run first."));
        assert_eq!(c.denied("ls --dry-run"), None);
    }

    #[test]
    fn an_unless_that_is_not_a_regex_is_an_error() {
        assert!(Commands::new(&[rule("x", Some("("), "y")]).is_err());
    }
}
