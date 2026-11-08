//! The rules that read more than one file: migrations, which read the tree
//! and its history, secret paths, which read names, and external commands.
//! Each stage runs them once, next to the per-file presets.

use std::collections::HashSet;
use std::path::Path;

use fairlead_core::config::Stage;

use crate::finding::Finding;
use crate::git::{self, Against, Staged};
use crate::migrations::Migrations;
use crate::rules::Guard;

/// What a stage adds to the presets' findings: findings that count,
/// warnings that never fail, and a note for a rule it couldn't run.
#[derive(Default)]
pub struct TreeRules {
    pub findings: Vec<Finding>,
    pub warnings: Vec<Finding>,
    pub notes: Vec<String>,
}

/// Lists HEAD's migrations for the write hook, which can't spare the
/// process; at commit that is the parent, which a refused commit keeps.
fn warm(m: &Migrations, root: &Path) {
    if m.immutable() {
        crate::head_paths::warm(root, &m.dirs());
    }
}

/// Runs each external rule for `stage`; a preset's missing tool is a note.
fn external(
    guard: &Guard,
    root: &Path,
    stage: Stage,
    files: &[String],
    out: &mut TreeRules,
) -> Result<Vec<Finding>, String> {
    let mut found = Vec::new();
    for e in guard.external.iter().filter(|e| e.runs_at(stage)) {
        let (more, note) = e.run_at(root, stage, files)?;
        found.extend(more);
        if let Some(note) = note {
            if !out.notes.iter().any(|n| n == note) {
                out.notes.push(note.to_string());
            }
        }
    }
    Ok(found)
}

/// The check stage: migrations over every tracked path, their history
/// against `base` (the flag, else `guard.migrations.base`), tracked secret
/// paths, and each external rule that runs at `check`, kept to tracked files.
pub fn tree(
    guard: &Guard,
    root: &Path,
    tracked: &[String],
    base: Option<&str>,
) -> Result<TreeRules, String> {
    let mut out = TreeRules::default();
    if let Some(m) = &guard.migrations {
        warm(m, root);
        out.findings.extend(m.prefix_findings(tracked));
        match base.or(m.base()) {
            Some(rev) if m.immutable() => {
                let at = git::merge_base(root, rev)?;
                let existed = git::paths_at(root, &at);
                let changes = git::changes(root, Against::Rev(&at))?;
                out.findings.extend(m.edit_findings(&changes, &existed, rev));
            }
            None if m.immutable() => out.notes.push(
                "migrations: without --base or guard.migrations.base, edits to existing migrations aren't checked here".into(),
            ),
            _ => {}
        }
    }
    if let Some(p) = &guard.paths {
        let read: Vec<String> = tracked
            .iter()
            .filter(|t| !guard.excluded(t))
            .cloned()
            .collect();
        let found = p.tracked(&read);
        out.findings.extend(found.findings);
        out.warnings.extend(found.warnings);
    }
    let known: HashSet<&str> = tracked.iter().map(String::as_str).collect();
    let found = external(guard, root, Stage::Check, &[], &mut out)?;
    out.findings.extend(
        found
            .into_iter()
            .filter(|f| known.contains(f.file.as_str())),
    );
    guard.cite(&mut out.findings);
    Ok(out)
}

/// The commit stage: a staged change to a migration that existed at the
/// base (the merge base with `guard.migrations.base`, else HEAD), a number
/// the staged tree newly shares or the base took since (with `fetch`), a
/// secret path the commit adds, and each external rule that runs at
/// `commit`, over the staged files, where every finding in them counts.
pub fn staged(guard: &Guard, root: &Path, staged: &[Staged]) -> Result<TreeRules, String> {
    let mut out = TreeRules::default();
    if let Some(m) = &guard.migrations {
        migrations_staged(m, root, staged, &mut out)?;
    }
    if let Some(p) = &guard.paths {
        let read: Vec<Staged> = staged
            .iter()
            .filter(|s| !guard.excluded(&s.path))
            .cloned()
            .collect();
        let found = p.staged(&read);
        out.findings.extend(found.findings);
        out.warnings.extend(found.warnings);
    }
    let paths: Vec<String> = staged.iter().map(|s| s.path.clone()).collect();
    let staged_set: HashSet<&str> = paths.iter().map(String::as_str).collect();
    if !paths.is_empty() {
        let found = external(guard, root, Stage::Commit, &paths, &mut out)?;
        out.findings.extend(
            found
                .into_iter()
                .filter(|f| staged_set.contains(f.file.as_str())),
        );
    }
    guard.cite(&mut out.findings);
    Ok(out)
}

fn migrations_staged(
    m: &Migrations,
    root: &Path,
    staged: &[Staged],
    out: &mut TreeRules,
) -> Result<(), String> {
    warm(m, root);
    // Before the first commit, or with the base ref not fetched, there's
    // no merge base; HEAD is the nearest thing that existed.
    let (rev, label) = match m.base().map(|b| (git::merge_base(root, b), b)) {
        Some((Ok(at), base)) => (at, base.to_string()),
        _ => ("HEAD".to_string(), "HEAD".to_string()),
    };
    let existed = git::paths_at(root, &rev);
    let changes = git::changes(root, Against::Index)?;
    out.findings
        .extend(m.edit_findings(&changes, &existed, &label));
    let head: Vec<String> = git::paths_at(root, "HEAD").into_iter().collect();
    let before = m.prefix_findings(&head);
    let index = git::tracked(root)?;
    let after = m.prefix_findings(&index);
    out.findings.extend(crate::added::added(&before, &after));
    let added: Vec<String> = staged
        .iter()
        .filter(|s| matches!(s.status, 'A' | 'C' | 'R') && m.covers(&s.path))
        .map(|s| s.path.clone())
        .collect();
    if m.fetches() && !added.is_empty() {
        out.findings
            .extend(taken(m, root, &index, &added, false, &mut out.notes));
    }
    Ok(())
}

/// Migrations in `added` whose number the base, fetched first unless a
/// fetch is fresh and `always` is false, gave to a file `ours` doesn't hold.
fn taken(
    m: &Migrations,
    root: &Path,
    ours: &[String],
    added: &[String],
    always: bool,
    notes: &mut Vec<String>,
) -> Vec<Finding> {
    let remote = crate::fetch::remote_for(root, m.base());
    if let Some(warning) = crate::fetch::refresh(root, &remote, always) {
        notes.push(warning);
    }
    let mine: HashSet<&str> = ours.iter().map(String::as_str).collect();
    let theirs: Vec<String> = git::paths_at(root, &remote.tracking)
        .into_iter()
        .filter(|p| m.covers(p) && !mine.contains(p.as_str()))
        .collect();
    m.taken_findings(added, &theirs, &remote.tracking)
}

/// Before a push: each migration the branch added since it left the base,
/// against the base fetched now, since another branch may have merged the
/// same number after this one last fetched.
pub fn push(guard: &Guard, root: &Path) -> TreeRules {
    let mut out = TreeRules::default();
    let Some(m) = guard.migrations.as_ref().filter(|m| m.fetches()) else {
        return out;
    };
    let remote = crate::fetch::remote_for(root, m.base());
    if let Some(warning) = crate::fetch::refresh(root, &remote, true) {
        out.notes.push(warning);
    }
    let ours: Vec<String> = git::paths_at(root, "HEAD").into_iter().collect();
    let then = git::merge_base(root, &remote.tracking)
        .map(|at| git::paths_at(root, &at))
        .unwrap_or_default();
    let added: Vec<String> = ours
        .iter()
        .filter(|p| m.covers(p) && !then.contains(*p))
        .cloned()
        .collect();
    let mine: HashSet<&str> = ours.iter().map(String::as_str).collect();
    let theirs: Vec<String> = git::paths_at(root, &remote.tracking)
        .into_iter()
        .filter(|p| m.covers(p) && !mine.contains(p.as_str()))
        .collect();
    out.findings = m.taken_findings(&added, &theirs, &remote.tracking);
    guard.cite(&mut out.findings);
    out
}
