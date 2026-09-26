//! The commit stage's entry in a lefthook config, added and removed as lines.
//! Parsing the YAML and writing it back would drop its comments and change
//! its layout, so the entry goes in by indentation, matching what's around it.

/// The command lefthook runs before each commit.
pub const RUN: &str = "fairlead guard check --staged";
const NAME: &str = "fairlead-guard";
/// The config names lefthook reads, in the order it looks for them.
pub const FILES: [&str; 4] = [
    "lefthook.yml",
    "lefthook.yaml",
    ".lefthook.yml",
    ".lefthook.yaml",
];

#[derive(Debug, PartialEq, Eq)]
pub enum Insert {
    Text(String),
    Already,
    /// A shape line insertion can't handle safely, and why.
    Refused(&'static str),
}

struct Lines {
    lines: Vec<String>,
    newline: &'static str,
    trailing: bool,
}

impl Lines {
    fn parse(text: &str) -> Lines {
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let trailing = text.ends_with('\n');
        let body = text.strip_suffix('\n').unwrap_or(text);
        let lines = if text.is_empty() {
            Vec::new()
        } else {
            body.split('\n')
                .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
                .collect()
        };
        Lines {
            lines,
            newline,
            trailing,
        }
    }

    fn join(&self) -> String {
        let mut out = self.lines.join(self.newline);
        if self.trailing && !self.lines.is_empty() {
            out.push_str(self.newline);
        }
        out
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Neither blank nor only a comment.
fn content(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && !t.starts_with('#')
}

/// `key:` with nothing after it but a comment.
fn is_header(line: &str, key: &str) -> bool {
    let Some(rest) = line
        .trim_start()
        .strip_prefix(key)
        .and_then(|r| r.strip_prefix(':'))
    else {
        return false;
    };
    let rest = rest.trim();
    rest.is_empty() || rest.starts_with('#')
}

/// `key:` followed by a value on the same line.
fn is_inline(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix(key)
        .and_then(|r| r.strip_prefix(':'))
        .is_some()
        && !is_header(line, key)
}

/// The end of the block that the line at `start` opens: the next content
/// line indented no deeper than it.
fn block_end(lines: &[String], start: usize) -> usize {
    let own = indent(&lines[start]);
    (start + 1..lines.len())
        .find(|&i| content(&lines[i]) && indent(&lines[i]) <= own)
        .unwrap_or(lines.len())
}

/// The indent of the first content line after `at` inside its block, if any.
fn child_indent(lines: &[String], at: usize) -> Option<usize> {
    let end = block_end(lines, at);
    (at + 1..end)
        .find(|&i| content(&lines[i]))
        .map(|i| indent(&lines[i]))
}

pub fn present(text: &str) -> bool {
    text.lines().any(|l| l.trim() == format!("run: {RUN}"))
}

/// The config with the entry added: under `pre-commit` `commands`, as a
/// `jobs` item where the config uses jobs, or as a new block at the end.
pub fn insert(text: &str) -> Insert {
    if present(text) {
        return Insert::Already;
    }
    let mut doc = Lines::parse(text);
    let lines = &doc.lines;
    let pre = lines
        .iter()
        .position(|l| indent(l) == 0 && (is_header(l, "pre-commit") || is_inline(l, "pre-commit")));
    let Some(pre) = pre else {
        if !doc.lines.is_empty() && !doc.trailing {
            doc.trailing = true;
        }
        doc.lines.extend([
            "pre-commit:".to_string(),
            "  commands:".to_string(),
            format!("    {NAME}:"),
            format!("      run: {RUN}"),
        ]);
        doc.trailing = true;
        return Insert::Text(doc.join());
    };
    if is_inline(&lines[pre], "pre-commit") {
        return Insert::Refused(
            "`pre-commit` is written on one line; add the command to it by hand",
        );
    }
    let end = block_end(lines, pre);
    let child = child_indent(lines, pre).unwrap_or(2);
    let step = child.max(1);
    let under = |key: &str| {
        (pre + 1..end).find(|&i| {
            indent(&lines[i]) == child && (is_header(&lines[i], key) || is_inline(&lines[i], key))
        })
    };
    let (at, new): (usize, Vec<String>) = if let Some(c) = under("commands") {
        if is_inline(&lines[c], "commands") {
            return Insert::Refused(
                "`commands` is written on one line; add the command to it by hand",
            );
        }
        let inner = child_indent(lines, c)
            .filter(|&i| i > child)
            .unwrap_or(child + step);
        let deeper = inner + (inner - child);
        (
            c + 1,
            vec![
                format!("{}{NAME}:", " ".repeat(inner)),
                format!("{}run: {RUN}", " ".repeat(deeper)),
            ],
        )
    } else if let Some(j) = under("jobs") {
        if is_inline(&lines[j], "jobs") {
            return Insert::Refused("`jobs` is written on one line; add the command to it by hand");
        }
        let item = child_indent(lines, j).unwrap_or(child + step);
        let pad = " ".repeat(item);
        (
            j + 1,
            vec![format!("{pad}- name: {NAME}"), format!("{pad}  run: {RUN}")],
        )
    } else {
        let pad = |n: usize| " ".repeat(n);
        (
            pre + 1,
            vec![
                format!("{}commands:", pad(child)),
                format!("{}{NAME}:", pad(child + step)),
                format!("{}run: {RUN}", pad(child + 2 * step)),
            ],
        )
    };
    doc.lines.splice(at..at, new);
    Insert::Text(doc.join())
}

/// The config without the entry, and without any `commands`, `jobs` or
/// `pre-commit` it leaves empty; None when the entry isn't there.
pub fn remove(text: &str) -> Option<String> {
    let mut doc = Lines::parse(text);
    let run = format!("run: {RUN}");
    let at = (0..doc.lines.len().saturating_sub(1)).find(|&i| {
        let (head, next) = (doc.lines[i].trim(), doc.lines[i + 1].trim());
        (head == format!("{NAME}:") || head == format!("- name: {NAME}")) && next == run
    })?;
    doc.lines.drain(at..at + 2);
    let mut i = at;
    while i > 0 {
        let header = i - 1;
        let line = &doc.lines[header];
        if !["commands", "jobs", "pre-commit"]
            .iter()
            .any(|k| is_header(line, k))
        {
            break;
        }
        let empty = doc.lines[header + 1..]
            .iter()
            .find(|l| content(l))
            .is_none_or(|l| indent(l) <= indent(line));
        if !empty {
            break;
        }
        doc.lines.remove(header);
        i = header;
    }
    if doc.lines.iter().all(|l| l.trim().is_empty()) {
        return Some(String::new());
    }
    Some(doc.join())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(insert: Insert) -> String {
        match insert {
            Insert::Text(t) => t,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_empty_or_new_config_gets_a_whole_block_and_loses_it_again() {
        let made = text(insert(""));
        assert_eq!(
            made,
            format!("pre-commit:\n  commands:\n    fairlead-guard:\n      run: {RUN}\n")
        );
        assert_eq!(remove(&made).unwrap(), "");
        let other = "# ours\npre-push:\n  commands:\n    test:\n      run: make test\n";
        let both = text(insert(other));
        assert!(both.starts_with(other));
        assert_eq!(remove(&both).unwrap(), other);
    }

    #[test]
    fn an_existing_commands_block_gets_the_entry_at_its_own_indent_with_comments_kept() {
        let config = "pre-commit:\n    parallel: true\n    commands:\n        # lint first\n        lint:\n            run: npm run lint\n";
        let added = text(insert(config));
        assert_eq!(
            added,
            format!("pre-commit:\n    parallel: true\n    commands:\n        fairlead-guard:\n            run: {RUN}\n        # lint first\n        lint:\n            run: npm run lint\n")
        );
        assert_eq!(remove(&added).unwrap(), config);
        assert_eq!(insert(&added), Insert::Already);
    }

    #[test]
    fn a_jobs_list_gets_an_item_and_a_bare_pre_commit_gets_commands() {
        let jobs = "pre-commit:\n  jobs:\n    - name: lint\n      run: npm run lint\n";
        let added = text(insert(jobs));
        assert!(
            added.contains(&format!(
                "  jobs:\n    - name: fairlead-guard\n      run: {RUN}\n    - name: lint"
            )),
            "{added}"
        );
        assert_eq!(remove(&added).unwrap(), jobs);
        let bare = "pre-commit:\n  parallel: true\nskip_output:\n  - meta\n";
        let added = text(insert(bare));
        assert!(added.starts_with(&format!("pre-commit:\n  commands:\n    fairlead-guard:\n      run: {RUN}\n  parallel: true\n")), "{added}");
        assert_eq!(remove(&added).unwrap(), bare);
    }

    #[test]
    fn crlf_and_a_missing_final_newline_are_kept() {
        let config = "pre-commit:\r\n  commands:\r\n    lint:\r\n      run: x";
        let added = text(insert(config));
        assert!(
            added.contains("\r\n    fairlead-guard:\r\n") && !added.ends_with('\n'),
            "{added:?}"
        );
        assert_eq!(remove(&added).unwrap(), config);
        let plain = "pre-push:\n  commands: {}";
        assert!(text(insert(plain)).starts_with("pre-push:\n  commands: {}\npre-commit:\n"));
    }

    #[test]
    fn a_one_line_value_is_refused_and_a_missing_entry_removes_nothing() {
        assert!(matches!(insert("pre-commit: {}\n"), Insert::Refused(_)));
        assert!(matches!(
            insert("pre-commit:\n  commands: {}\n"),
            Insert::Refused(_)
        ));
        assert_eq!(
            remove("pre-commit:\n  commands:\n    lint:\n      run: x\n"),
            None
        );
    }
}
