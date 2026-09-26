//! The rules that read more than one file: migrations, which read the tree
//! and its history, and external commands. Each stage runs them once, next
//! to the per-file presets.

use std::collections::HashSet;
use std::path::Path;

use fairlead_core::config::Stage;

use crate::finding::Finding;
use crate::git::{self, Against};
use crate::rules::Guard;

/// What the check stage adds to the presets' findings, and a note for a
/// rule it couldn't run.
pub struct TreeRules {
    pub findings: Vec<Finding>,
    pub notes: Vec<String>,
}

/// The check stage: migrations over every tracked path, their history
/// against `base` (the flag, else `guard.migrations.base`), and each
/// external rule that runs at `check`.
pub fn tree(
    guard: &Guard,
    root: &Path,
    tracked: &[String],
    base: Option<&str>,
) -> Result<TreeRules, String> {
    let mut findings = Vec::new();
    let mut notes = Vec::new();
    if let Some(m) = &guard.migrations {
        findings.extend(m.prefix_findings(tracked));
        match base.or(m.base()) {
            Some(rev) if m.immutable() => {
                let at = git::merge_base(root, rev)?;
                let existed = git::paths_at(root, &at);
                let changes = git::changes(root, Against::Rev(&at))?;
                findings.extend(m.edit_findings(&changes, &existed, rev));
            }
            None if m.immutable() => notes.push(
                "migrations: without --base or guard.migrations.base, edits to existing migrations aren't checked here".into(),
            ),
            _ => {}
        }
    }
    for e in guard.external.iter().filter(|e| e.runs_at(Stage::Check)) {
        findings.extend(e.run(root, &[])?);
    }
    guard.cite(&mut findings);
    Ok(TreeRules { findings, notes })
}

/// The commit stage: a staged change to a migration that existed at the
/// base (the merge base with `guard.migrations.base`, else HEAD), a number
/// the staged tree newly shares, and each external rule that runs at
/// `commit`, over the staged files, where every finding in them counts.
pub fn staged(guard: &Guard, root: &Path, staged: &[String]) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    if let Some(m) = &guard.migrations {
        // Before the first commit, or with the base ref not fetched, there's
        // no merge base; HEAD is the nearest thing that existed.
        let (rev, label) = match m.base().map(|b| (git::merge_base(root, b), b)) {
            Some((Ok(at), base)) => (at, base.to_string()),
            _ => ("HEAD".to_string(), "HEAD".to_string()),
        };
        let existed = git::paths_at(root, &rev);
        let changes = git::changes(root, Against::Index)?;
        findings.extend(m.edit_findings(&changes, &existed, &label));
        let head: Vec<String> = git::paths_at(root, "HEAD").into_iter().collect();
        let before = m.prefix_findings(&head);
        let after = m.prefix_findings(&git::tracked(root)?);
        findings.extend(crate::added::added(&before, &after));
    }
    let staged_set: HashSet<&str> = staged.iter().map(String::as_str).collect();
    for e in guard.external.iter().filter(|e| e.runs_at(Stage::Commit)) {
        if staged.is_empty() {
            break;
        }
        let found = e.run(root, staged)?;
        findings.extend(
            found
                .into_iter()
                .filter(|f| staged_set.contains(f.file.as_str())),
        );
    }
    guard.cite(&mut findings);
    Ok(findings)
}
