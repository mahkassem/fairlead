//! A `diff = true` external rule: the tool runs on the head and on the
//! base, in a temporary worktree removed afterwards, and only what the head
//! adds counts. Findings match by file, across a rename, and by message with
//! numbers taken out, so code that moved isn't new.

use std::collections::HashMap;
use std::path::Path;

use crate::external::External;
use crate::finding::Finding;

/// What a diff rule compares with: a commit, the change's renames as new
/// path to old, and the files a tool that takes `{files}` runs on.
pub struct Base {
    pub rev: String,
    pub renames: HashMap<String, String>,
    pub files: Vec<String>,
}

/// A message with each run of digits as `#` and spaces collapsed, so a
/// line or column number inside it doesn't make an old finding new.
pub fn normalize(message: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for word in message.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in word.chars() {
            if c.is_ascii_digit() {
                if !digits {
                    out.push('#');
                }
                digits = true;
            } else {
                digits = false;
                out.extend(c.to_lowercase());
            }
        }
        digits = false;
    }
    out
}

/// The head's findings the base doesn't have as many of, by file and
/// normalized message; the head's path is read as the base's across a rename.
pub fn added(
    head: Vec<Finding>,
    base: &[Finding],
    renames: &HashMap<String, String>,
) -> Vec<Finding> {
    let mut before: HashMap<(String, String), usize> = HashMap::new();
    for f in base {
        *before
            .entry((f.file.clone(), normalize(&f.message)))
            .or_default() += 1;
    }
    head.into_iter()
        .filter(|f| {
            let file = renames.get(&f.file).unwrap_or(&f.file).clone();
            match before.get_mut(&(file, normalize(&f.message))) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    false
                }
                _ => true,
            }
        })
        .collect()
}

/// Runs the rule on the head and, when it found anything, on the base.
/// A base the tool can't run on is a note, and every head finding counts.
pub fn run(
    e: &External,
    root: &Path,
    base: &Base,
) -> (Result<Vec<Finding>, String>, Option<String>) {
    let head = match e.run(root, &base.files) {
        Ok(found) if found.is_empty() => return (Ok(found), None),
        Ok(found) => found,
        Err(err) => return (Err(err), None),
    };
    let old: Vec<String> = base
        .files
        .iter()
        .map(|f| base.renames.get(f).unwrap_or(f).clone())
        .collect();
    let checkout = match e.takes_files() {
        true => crate::worktree::check_out_files(root, &base.rev, &old),
        false => crate::worktree::check_out(root, &base.rev),
    };
    let short: String = base.rev.chars().take(7).collect();
    let checkout = match checkout {
        Ok(c) => c,
        Err(err) => {
            let note = format!(
                "{}: couldn't check out the base {short}, so every finding counts: {err}",
                e.id
            );
            return (Ok(head), Some(note));
        }
    };
    let present: Vec<String> = old
        .into_iter()
        .filter(|f| checkout.dir.join(f).is_file())
        .collect();
    if e.takes_files() && present.is_empty() {
        return (Ok(head), None);
    }
    match e.run(&checkout.dir, &present) {
        Ok(before) => (Ok(added(head, &before, &base.renames)), None),
        Err(err) => {
            let note = format!(
                "{}: the tool failed on the base {short}, so every finding counts: {err}",
                e.id
            );
            (Ok(head), Some(note))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(file: &str, line: u32, message: &str) -> Finding {
        Finding {
            file: file.into(),
            line,
            rule: "lint",
            message: message.into(),
            anchor: String::new(),
            measure: None,
        }
    }

    #[test]
    fn numbers_and_spacing_inside_a_message_dont_make_it_new() {
        assert_eq!(
            normalize("Unused  var `x` (col 12)"),
            "unused var `x` (col #)"
        );
        assert_eq!(normalize("line 3: x1 and x22"), "line #: x# and x#");
    }

    #[test]
    fn only_what_the_head_adds_counts_moved_and_renamed_findings_dont() {
        let base = [
            found("a.ts", 3, "unused var at 3:4"),
            found("old.ts", 1, "no-any"),
        ];
        let head = vec![
            found("a.ts", 40, "unused var at 40:4"),
            found("a.ts", 41, "unused var at 41:4"),
            found("new.ts", 9, "no-any"),
            found("b.ts", 2, "no-any"),
        ];
        let renames = HashMap::from([("new.ts".to_string(), "old.ts".to_string())]);
        let left: Vec<String> = added(head, &base, &renames)
            .iter()
            .map(|f| format!("{}:{}", f.file, f.line))
            .collect();
        assert_eq!(left, ["a.ts:41", "b.ts:2"]);
    }
}
