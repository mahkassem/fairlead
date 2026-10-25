//! `plan.run_all` compiled: which change to a path runs everything, and the
//! kind of change that did when the entry that caught it names kinds.

use std::collections::BTreeMap;

use fairlead_core::config;
use fairlead_core::plan::{Change, Status};

use crate::pattern::Pattern;

pub struct RunAll {
    entries: Vec<Entry>,
}

struct Entry {
    globs: Vec<Pattern>,
    /// `None` runs everything for any kind of change.
    kinds: Option<Vec<Status>>,
}

impl RunAll {
    pub fn new(entries: &[config::RunAll]) -> Result<Self, String> {
        let entries = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                Ok(Entry {
                    globs: crate::planner::patterns(entry.globs())?,
                    kinds: entry
                        .kinds()
                        .map_err(|e| format!("plan.run_all[{i}].on: {e}"))?,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(RunAll { entries })
    }

    /// Whether a `status` change at `path` runs everything: `None` if not,
    /// else the kind to name, which only an entry with `on` gives.
    pub fn hit(&self, path: &str, status: Status) -> Option<Option<Status>> {
        self.entries
            .iter()
            .find(|e| {
                e.kinds.as_ref().is_none_or(|k| k.contains(&status))
                    && e.globs.iter().any(|g| g.is_match(path))
            })
            .map(|e| e.kinds.as_ref().map(|_| status))
    }

    /// Whether editing `path` runs everything, for a file whose meaning
    /// changes in place, such as one importing a package whose version moved.
    pub fn on_edit(&self, path: &str) -> bool {
        self.hit(path, Status::Modified).is_some()
    }
}

/// Each changed path with the kinds of change at it. A rename is `renamed`
/// at its old path and its new one, as both count as changed.
pub fn kinds_by_path(changes: &[Change]) -> BTreeMap<&str, Vec<Status>> {
    let mut out: BTreeMap<&str, Vec<Status>> = BTreeMap::new();
    for change in changes {
        let paths = std::iter::once(&change.path).chain(&change.from);
        for path in paths {
            let kinds = out.entry(path.as_str()).or_default();
            if !kinds.contains(&change.status) {
                kinds.push(change.status);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled(toml_text: &str) -> RunAll {
        let config: config::Config = toml::from_str(toml_text).unwrap();
        RunAll::new(config.plan.run_all.items()).unwrap()
    }

    #[test]
    fn a_glob_hits_every_kind_and_names_none() {
        let run_all = compiled("[plan]\nrun_all = [\"ci/**\"]\n");
        for kind in Status::ALL {
            assert_eq!(run_all.hit("ci/a.yml", kind), Some(None));
        }
        assert_eq!(run_all.hit("src/a.ts", Status::Modified), None);
    }

    #[test]
    fn an_entry_with_on_hits_only_its_kinds_and_names_the_kind() {
        let run_all = compiled(
            "[plan]\nrun_all = [{ paths = [\"db/**\"], on = [\"modified\", \"deleted\"] }]\n",
        );
        assert_eq!(run_all.hit("db/1.sql", Status::Added), None);
        assert_eq!(run_all.hit("db/1.sql", Status::Renamed), None);
        assert_eq!(
            run_all.hit("db/1.sql", Status::Modified),
            Some(Some(Status::Modified))
        );
        assert!(run_all.on_edit("db/1.sql"));
    }

    #[test]
    fn the_first_entry_that_hits_decides_what_is_named() {
        let run_all =
            compiled("[plan]\nrun_all = [{ paths = \"db/**\", on = [\"modified\"] }, \"db/**\"]\n");
        assert_eq!(
            run_all.hit("db/1.sql", Status::Modified),
            Some(Some(Status::Modified))
        );
        assert_eq!(run_all.hit("db/1.sql", Status::Added), Some(None));
    }

    #[test]
    fn an_unknown_kind_fails_to_compile() {
        let config: config::Config =
            toml::from_str("[plan]\nrun_all = [{ paths = \"db/**\", on = [\"edit\"] }]\n").unwrap();
        let err = RunAll::new(config.plan.run_all.items()).err().unwrap();
        assert!(err.starts_with("plan.run_all[0].on: `edit`"), "{err}");
    }

    #[test]
    fn a_rename_counts_at_both_paths() {
        let changes = vec![
            Change {
                path: "new.sql".into(),
                status: Status::Renamed,
                from: Some("old.sql".into()),
            },
            Change {
                path: "old.sql".into(),
                status: Status::Added,
                from: None,
            },
        ];
        let kinds = kinds_by_path(&changes);
        assert_eq!(kinds["new.sql"], [Status::Renamed]);
        assert_eq!(kinds["old.sql"], [Status::Renamed, Status::Added]);
    }
}
