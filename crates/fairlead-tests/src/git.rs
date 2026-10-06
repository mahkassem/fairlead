//! What changed, from git: the diff between a base commit and the working
//! tree, plus untracked files, with renames as both paths. The working tree
//! is always the head, which in CI is the checked-out commit.

use std::path::Path;
use std::process::Command;

pub use fairlead_core::plan::{Change, Status};

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A file's text at `rev`, or `None` if it didn't exist there.
pub fn file_at(root: &Path, rev: &str, path: &str) -> Option<String> {
    git(
        root,
        &["show", "--end-of-options", &format!("{rev}:{path}")],
    )
    .ok()
}

/// The commit a plan compares against: the merge base of `base` and HEAD.
pub fn merge_base(root: &Path, base: &str) -> Result<String, String> {
    Ok(git(root, &["merge-base", base, "HEAD"])?.trim().to_string())
}

/// The default base: the remote's default branch, else `main`, else `master`.
pub fn default_base(root: &Path) -> Result<String, String> {
    let candidates = [
        "origin/HEAD",
        "origin/main",
        "origin/master",
        "main",
        "master",
    ];
    candidates
        .iter()
        .find(|c| {
            git(
                root,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{c}^{{commit}}"),
                ],
            )
            .is_ok()
        })
        .map(|c| c.to_string())
        .ok_or_else(|| "no base branch found; pass --base".to_string())
}

/// Changes from `base` (a commit) to the working tree, untracked files included.
pub fn changes(root: &Path, base: &str) -> Result<Vec<Change>, String> {
    let diff = git(
        root,
        &[
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            "--no-ext-diff",
            base,
            "--",
        ],
    )?;
    let mut changes = parse_name_status(&diff);
    let untracked = git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    changes.extend(
        untracked
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(|p| Change {
                path: p.to_string(),
                status: Status::Added,
                from: None,
            }),
    );
    changes.sort();
    changes.dedup();
    Ok(changes)
}

/// HEAD's commit id.
pub fn head(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
}

/// The branch HEAD is on, or `None` when it's detached.
pub fn branch(root: &Path) -> Option<String> {
    git(root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The last `n` commits that touched `path`, as `hash date subject`.
pub fn recent_commits(root: &Path, path: &str, n: usize) -> Vec<String> {
    let count = format!("-{n}");
    git(
        root,
        &[
            "log",
            &count,
            "--format=%h %ad %s",
            "--date=short",
            "--",
            path,
        ],
    )
    .map(|out| out.lines().map(str::to_string).collect())
    .unwrap_or_default()
}

/// Whether a boolean git setting is true for the repository, as git reads it.
pub fn config_true(root: &Path, key: &str) -> bool {
    git(root, &["config", "--get", key]).is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "on" | "1"
        )
    })
}

/// The tree the plan was made from: HEAD's tree id when the working tree
/// matches it, else `None`.
pub fn clean_tree_id(root: &Path) -> Option<String> {
    let status = git(root, &["status", "--porcelain", "-z"]).ok()?;
    if !status.is_empty() {
        return None;
    }
    git(root, &["rev-parse", "HEAD^{tree}"])
        .ok()
        .map(|s| s.trim().to_string())
}

/// `git diff --name-status -z` output: a status, then one path, or two for a
/// rename or copy.
pub fn parse_name_status(raw: &str) -> Vec<Change> {
    let mut fields = raw.split('\0').filter(|f| !f.is_empty());
    let mut out = Vec::new();
    while let Some(code) = fields.next() {
        let kind = code.chars().next().unwrap_or('M');
        match kind {
            'R' | 'C' => {
                let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                    break;
                };
                out.push(Change {
                    path: new.to_string(),
                    status: if kind == 'R' {
                        Status::Renamed
                    } else {
                        Status::Added
                    },
                    from: (kind == 'R').then(|| old.to_string()),
                });
            }
            _ => {
                let Some(path) = fields.next() else {
                    break;
                };
                let status = match kind {
                    'A' => Status::Added,
                    'D' => Status::Deleted,
                    _ => Status::Modified,
                };
                out.push(Change {
                    path: path.to_string(),
                    status,
                    from: None,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status_reads_renames_as_both_paths() {
        let raw = "M\0a.ts\0R087\0old/b.ts\0new/b.ts\0D\0gone.ts\0A\0added.ts\0";
        let changes = parse_name_status(raw);
        assert_eq!(changes.len(), 4);
        assert_eq!(changes[1].path, "new/b.ts");
        assert_eq!(changes[1].from.as_deref(), Some("old/b.ts"));
        assert_eq!(changes[2].status, Status::Deleted);
        assert_eq!(changes[3].status, Status::Added);
    }
}
