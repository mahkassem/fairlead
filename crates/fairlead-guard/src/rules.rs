use std::collections::BTreeSet;

use fairlead_core::config;
use fairlead_core::pattern::Pattern;

use crate::finding::{self, Finding};
use crate::size::FileLength;

/// A file as a rule reads it: its path from the project root, and its text.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

pub trait Rule: Send + Sync {
    fn id(&self) -> &'static str;
    /// Counted against the baseline rather than failing on any finding.
    fn ratcheted(&self) -> bool;
    fn applies(&self, path: &str) -> bool;
    fn check(&self, source: Source<'_>) -> Vec<Finding>;
}

/// Files a rule reads: any of `files`, none of `exclude`.
#[derive(Debug)]
pub(crate) struct Scope {
    files: Vec<Pattern>,
    exclude: Vec<Pattern>,
}

impl Scope {
    pub(crate) fn new(files: &[String], exclude: &[String]) -> Result<Scope, String> {
        let compile = |globs: &[String]| -> Result<Vec<Pattern>, String> {
            globs.iter().map(|g| Pattern::new(g)).collect()
        };
        Ok(Scope {
            files: compile(files)?,
            exclude: compile(exclude)?,
        })
    }

    pub(crate) fn contains(&self, path: &str) -> bool {
        self.files.iter().any(|p| p.is_match(path))
            && !self.exclude.iter().any(|p| p.is_match(path))
    }
}

/// Every configured rule. A preset that isn't configured adds none.
pub struct Guard {
    rules: Vec<Box<dyn Rule>>,
    exclude: Vec<Pattern>,
}

impl Guard {
    pub fn new(config: &config::Guard) -> Result<Guard, String> {
        let mut rules: Vec<Box<dyn Rule>> = Vec::new();
        if let Some(size) = &config.size {
            let scope = Scope::new(size.files.items(), size.exclude.items())?;
            if let Some(limit) = size.file_lines {
                rules.push(Box::new(FileLength::new(scope, limit, size.ratchet)));
            }
        }
        let exclude = config
            .exclude
            .items()
            .iter()
            .map(|g| Pattern::new(g))
            .collect::<Result<_, _>>()?;
        Ok(Guard { rules, exclude })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Whether any rule reads this path.
    pub fn reads(&self, path: &str) -> bool {
        !self.exclude.iter().any(|p| p.is_match(path)) && self.rules.iter().any(|r| r.applies(path))
    }

    pub fn ratcheted(&self) -> BTreeSet<&'static str> {
        self.rules
            .iter()
            .filter(|r| r.ratcheted())
            .map(|r| r.id())
            .collect()
    }

    /// Every finding in one file, in reading order.
    pub fn lint(&self, source: Source<'_>) -> Vec<Finding> {
        if !self.reads(source.path) {
            return Vec::new();
        }
        let mut found: Vec<Finding> = self
            .rules
            .iter()
            .filter(|r| r.applies(source.path))
            .flat_map(|r| r.check(source))
            .collect();
        finding::sort(&mut found);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(file_lines: Option<u32>, ratchet: bool) -> config::Guard {
        config::Guard {
            size: Some(config::SizeRules {
                files: vec!["src/**".to_string()].into(),
                exclude: vec!["src/gen/**".to_string()].into(),
                file_lines,
                ratchet,
            }),
            ..config::Guard::default()
        }
    }

    #[test]
    fn a_preset_that_is_not_configured_adds_no_rule() {
        assert!(Guard::new(&config::Guard::default()).unwrap().is_empty());
        assert!(Guard::new(&size(None, true)).unwrap().is_empty());
    }

    #[test]
    fn a_rule_reads_only_its_files_and_never_an_excluded_one() {
        let mut config = size(Some(1), true);
        config.exclude = vec!["src/vendor/**".to_string()].into();
        let guard = Guard::new(&config).unwrap();
        assert!(guard.reads("src/a.ts"));
        assert!(!guard.reads("lib/a.ts"));
        assert!(!guard.reads("src/gen/a.ts"));
        assert!(!guard.reads("src/vendor/a.ts"));
        let two_lines = Source {
            path: "src/vendor/a.ts",
            text: "a\nb\n",
        };
        assert!(guard.lint(two_lines).is_empty());
    }

    #[test]
    fn ratchet_is_the_presets_choice() {
        let ratcheted = Guard::new(&size(Some(1), true)).unwrap().ratcheted();
        assert!(ratcheted.contains("file-length"));
        assert!(Guard::new(&size(Some(1), false))
            .unwrap()
            .ratcheted()
            .is_empty());
    }
}
