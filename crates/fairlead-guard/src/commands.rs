//! `[[guard.commands]]`: shell commands an agent may not run, each with the
//! reason it is told. Read by the write stage's Bash hook.

use fairlead_core::config::CommandRule;
use regex::Regex;

#[derive(Default)]
pub struct Commands {
    rules: Vec<(Regex, String)>,
}

impl Commands {
    pub fn new(rules: &[CommandRule]) -> Result<Commands, String> {
        let rules = rules
            .iter()
            .map(|r| {
                Ok((
                    Regex::new(&r.matches).map_err(|e| e.to_string())?,
                    r.reason.clone(),
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(Commands { rules })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The reason the first matching rule gives, if any matches.
    pub fn denied(&self, command: &str) -> Option<&str> {
        self.rules
            .iter()
            .find(|(re, _)| re.is_match(command))
            .map(|(_, reason)| reason.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_matching_rule_gives_its_reason() {
        let rule = |m: &str, r: &str| CommandRule {
            matches: m.into(),
            reason: r.into(),
        };
        let c = Commands::new(&[
            rule(r"(^|\s)git stash(\s|$)", "Commit instead."),
            rule(r"rm -rf /", "Never."),
        ])
        .unwrap();
        assert_eq!(c.denied("cd x && git stash pop"), Some("Commit instead."));
        assert_eq!(c.denied("git stashed"), None);
        assert_eq!(c.denied("ls"), None);
    }
}
