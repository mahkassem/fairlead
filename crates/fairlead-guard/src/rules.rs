use std::cell::OnceCell;
use std::collections::BTreeSet;

use fairlead_core::config;
use fairlead_core::pattern::Pattern;
use fairlead_lang::tree_sitter::Tree;

use crate::comments::Comments;
use crate::finding::{self, Finding};
use crate::size::Size;

/// A file as the presets read it: its path from the project root, its text,
/// and its syntax tree, parsed at most once however many presets ask.
pub struct Source<'a> {
    pub path: &'a str,
    pub text: &'a str,
    tree: OnceCell<Option<Tree>>,
}

impl<'a> Source<'a> {
    /// A leading byte-order mark is dropped, so line 1 reads as it looks.
    pub fn new(path: &'a str, text: &'a str) -> Source<'a> {
        Source {
            path,
            text: text.strip_prefix('\u{feff}').unwrap_or(text),
            tree: OnceCell::new(),
        }
    }

    /// None for a file that isn't JavaScript or TypeScript.
    pub fn tree(&self) -> Option<&Tree> {
        self.tree
            .get_or_init(|| fairlead_lang::extract::parse(self.path, self.text.as_bytes()))
            .as_ref()
    }
}

/// A family of rules configured together, such as `[guard.comments]`.
pub trait Preset: Send + Sync {
    /// The rule ids it reports that count against the baseline.
    fn ratcheted(&self) -> Vec<&'static str>;
    fn applies(&self, path: &str) -> bool;
    fn check(&self, source: &Source<'_>) -> Vec<Finding>;
}

/// Files a preset reads: any of `files`, none of `exclude`.
#[derive(Debug)]
pub(crate) struct Scope {
    files: Vec<Pattern>,
    exclude: Vec<Pattern>,
}

pub(crate) fn compile(globs: &[String]) -> Result<Vec<Pattern>, String> {
    globs.iter().map(|g| Pattern::new(g)).collect()
}

impl Scope {
    pub(crate) fn new(files: &[String], exclude: &[String]) -> Result<Scope, String> {
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

/// Every configured preset. One that isn't configured adds nothing.
pub struct Guard {
    presets: Vec<Box<dyn Preset>>,
    exclude: Vec<Pattern>,
    cite: std::collections::BTreeMap<String, String>,
}

impl Guard {
    pub fn new(config: &config::Guard) -> Result<Guard, String> {
        let mut presets: Vec<Box<dyn Preset>> = Vec::new();
        if let Some(size) = &config.size {
            presets.push(Box::new(Size::new(size)?));
        }
        if let Some(comments) = &config.comments {
            presets.push(Box::new(Comments::new(comments)?));
        }
        Ok(Guard {
            presets,
            exclude: compile(config.exclude.items())?,
            cite: config.cite.clone(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }

    /// Whether any preset reads this path.
    pub fn reads(&self, path: &str) -> bool {
        !self.exclude.iter().any(|p| p.is_match(path))
            && self.presets.iter().any(|r| r.applies(path))
    }

    pub fn ratcheted(&self) -> BTreeSet<&'static str> {
        self.presets.iter().flat_map(|p| p.ratcheted()).collect()
    }

    /// Every finding in one file, in reading order.
    pub fn lint(&self, source: &Source<'_>) -> Vec<Finding> {
        if !self.reads(source.path) {
            return Vec::new();
        }
        let mut found: Vec<Finding> = self
            .presets
            .iter()
            .filter(|p| p.applies(source.path))
            .flat_map(|p| p.check(source))
            .collect();
        for f in &mut found {
            if let Some(note) = self.cite.get(f.rule) {
                f.message = format!("{} ({note})", f.message);
            }
        }
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
                ..config::SizeRules::default()
            }),
            ..config::Guard::default()
        }
    }

    #[test]
    fn a_preset_that_is_not_configured_adds_nothing() {
        assert!(Guard::new(&config::Guard::default()).unwrap().is_empty());
    }

    #[test]
    fn a_preset_reads_only_its_files_and_never_an_excluded_one() {
        let mut config = size(Some(1), true);
        config.exclude = vec!["src/vendor/**".to_string()].into();
        let guard = Guard::new(&config).unwrap();
        assert!(guard.reads("src/a.ts"));
        assert!(!guard.reads("lib/a.ts"));
        assert!(!guard.reads("src/gen/a.ts"));
        assert!(!guard.reads("src/vendor/a.ts"));
        assert!(guard
            .lint(&Source::new("src/vendor/a.ts", "a\nb\n"))
            .is_empty());
    }

    #[test]
    fn ratchet_is_the_presets_choice() {
        let ratcheted = Guard::new(&size(Some(1), true)).unwrap().ratcheted();
        assert!(ratcheted.contains("file-length") && ratcheted.contains("function-length"));
        assert!(Guard::new(&size(Some(1), false))
            .unwrap()
            .ratcheted()
            .is_empty());
    }

    #[test]
    fn a_cite_is_appended_to_its_rules_messages() {
        let mut config = size(Some(1), true);
        config
            .cite
            .insert("file-length".into(), "guide, section 6".into());
        let found = Guard::new(&config)
            .unwrap()
            .lint(&Source::new("src/a.ts", "a\nb\n"));
        assert_eq!(
            found[0].message,
            "file is 2 lines, over 1 (guide, section 6)"
        );
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_a_header_comment() {
        assert_eq!(Source::new("a.ts", "\u{feff}// a\n").text, "// a\n");
    }

    #[test]
    fn the_tree_is_parsed_only_for_javascript_and_typescript() {
        assert!(Source::new("a.ts", "const a = 1\n").tree().is_some());
        assert!(Source::new("a.sql", "select 1;\n").tree().is_none());
    }
}
