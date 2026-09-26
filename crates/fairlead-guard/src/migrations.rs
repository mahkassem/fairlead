//! `[guard.migrations]`: a migration that already exists doesn't change,
//! move or go, since a database that ran it won't run it again; and each
//! one's leading number is unique, so the order they run in is clear.

use std::collections::{BTreeMap, HashSet};

use fairlead_core::config;
use fairlead_core::pattern::Pattern;

use crate::finding::Finding;
use crate::git::Change;
use crate::rules::compile;

pub struct Migrations {
    files: Vec<Pattern>,
    immutable: bool,
    base: Option<String>,
    unique: bool,
    allow: Vec<HashSet<String>>,
}

fn finding(path: &str, rule: &'static str, message: String) -> Finding {
    Finding {
        file: path.to_string(),
        line: 1,
        rule,
        message,
        anchor: String::new(),
        measure: None,
    }
}

/// A file name's leading digits, if it starts with any.
fn number(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next()?;
    let end = name
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(name.len());
    (end > 0).then(|| &name[..end])
}

fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Migrations {
    pub fn new(c: &config::Migrations) -> Result<Migrations, String> {
        let allow = c
            .unique_prefix
            .as_ref()
            .map(|u| {
                u.allow
                    .iter()
                    .map(|g| g.iter().cloned().collect())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Migrations {
            files: compile(c.files.items())?,
            immutable: c.immutable,
            base: c.base.clone(),
            unique: c.unique_prefix.is_some(),
            allow,
        })
    }

    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    pub fn immutable(&self) -> bool {
        self.immutable
    }

    pub fn covers(&self, path: &str) -> bool {
        self.files.iter().any(|p| p.is_match(path))
    }

    /// Each migration that shares its number with another, unless every
    /// file sharing it is in one allowed group.
    pub fn prefix_findings(&self, paths: &[String]) -> Vec<Finding> {
        if !self.unique {
            return Vec::new();
        }
        let mut by_number: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for path in paths.iter().filter(|p| self.covers(p)) {
            if let Some(n) = number(path) {
                by_number.entry(n).or_default().push(path);
            }
        }
        let mut out = Vec::new();
        for (n, group) in by_number.into_iter().filter(|(_, g)| g.len() > 1) {
            let names: HashSet<String> = group.iter().map(|p| name(p).to_string()).collect();
            if self.allow.iter().any(|allowed| names.is_subset(allowed)) {
                continue;
            }
            for path in &group {
                let others: Vec<&str> = group
                    .iter()
                    .filter(|p| *p != path)
                    .map(|p| name(p))
                    .collect();
                out.push(finding(
                    path,
                    "migration-prefix",
                    format!("shares its number {n} with {}", others.join(", ")),
                ));
            }
        }
        out
    }

    /// Each change to a migration that existed at the base: `existed`.
    pub fn edit_findings(
        &self,
        changes: &[Change],
        existed: &HashSet<String>,
        base: &str,
    ) -> Vec<Finding> {
        if !self.immutable {
            return Vec::new();
        }
        let old = |c: &Change| c.before.clone().unwrap_or_else(|| c.path.clone());
        changes
            .iter()
            .filter(|c| matches!(c.status, 'M' | 'R' | 'D' | 'T'))
            .filter(|c| self.covers(&old(c)) && existed.contains(&old(c)))
            .map(|c| {
                let what = match c.status {
                    'R' => format!("moves {} away from where it existed at {base}", old(c)),
                    'D' => format!("deletes a migration that existed at {base}"),
                    _ => format!("changes a migration that existed at {base}"),
                };
                finding(&c.path, "migration-edit", what)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migrations(allow: Vec<Vec<&str>>) -> Migrations {
        Migrations::new(&config::Migrations {
            files: vec!["db/*.sql".to_string()].into(),
            unique_prefix: Some(config::UniquePrefix {
                allow: allow
                    .into_iter()
                    .map(|g| g.into_iter().map(String::from).collect())
                    .collect(),
            }),
            ..config::Migrations::default()
        })
        .unwrap()
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_shared_number_is_a_finding_for_each_file_unless_the_group_is_allowed() {
        let m = migrations(vec![vec!["002_a.sql", "002_b.sql"]]);
        let found = m.prefix_findings(&paths(&[
            "db/001_x.sql",
            "db/001_y.sql",
            "db/002_a.sql",
            "db/002_b.sql",
            "db/003.sql",
            "db/notes.sql",
        ]));
        let lines: Vec<String> = found.iter().map(|f| f.to_string()).collect();
        assert_eq!(
            lines,
            [
                "db/001_x.sql:1 migration-prefix: shares its number 001 with 001_y.sql",
                "db/001_y.sql:1 migration-prefix: shares its number 001 with 001_x.sql"
            ]
        );
    }

    #[test]
    fn changing_moving_or_deleting_a_migration_that_existed_is_a_finding() {
        let m = migrations(vec![]);
        let change = |status, before: Option<&str>, path: &str| Change {
            status,
            before: before.map(String::from),
            path: path.into(),
        };
        let changes = [
            change('M', None, "db/001.sql"),
            change('R', Some("db/002.sql"), "db/old/002.sql"),
            change('D', None, "db/003.sql"),
            change('M', None, "db/004.sql"),
            change('A', None, "db/005.sql"),
            change('M', None, "src/a.ts"),
        ];
        let existed: HashSet<String> = ["db/001.sql", "db/002.sql", "db/003.sql", "src/a.ts"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let found: Vec<(String, String)> = m
            .edit_findings(&changes, &existed, "main")
            .into_iter()
            .map(|f| (f.file, f.message))
            .collect();
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[1].0, "db/old/002.sql");
        assert!(found[2].1.starts_with("deletes"));
    }
}
