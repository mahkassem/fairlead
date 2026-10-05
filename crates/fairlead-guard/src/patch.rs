//! Codex's `apply_patch` envelope, read the way Codex applies it, so the
//! write stage can lint each file as the patch would leave it. A patch that
//! doesn't parse or apply is let through: Codex refuses it on its own.

use crate::edit::Rebuilt;

/// One file the patch touches.
#[derive(Debug, PartialEq, Eq)]
pub struct FilePatch {
    pub path: String,
    pub change: Change,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Add(String),
    Delete,
    Update {
        to: Option<String>,
        chunks: Vec<Chunk>,
    },
}

/// One `@@` section: lines to find after an optional context line, and what
/// replaces them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Chunk {
    pub context: Option<String>,
    pub old: Vec<String>,
    pub new: Vec<String>,
    pub at_end: bool,
}

const BEGIN: &str = "*** Begin Patch";
const END: &str = "*** End Patch";
const ADD: &str = "*** Add File: ";
const DELETE: &str = "*** Delete File: ";
const UPDATE: &str = "*** Update File: ";
const MOVE: &str = "*** Move to: ";
const AT_END: &str = "*** End of File";

/// The files a patch touches, or why it isn't one Codex would accept.
pub fn parse(text: &str) -> Result<Vec<FilePatch>, &'static str> {
    let mut lines: Vec<&str> = text.trim().lines().collect();
    // Codex also takes the body of a heredoc the model wrapped it in.
    if lines.len() >= 4
        && matches!(lines[0], "<<EOF" | "<<'EOF'" | "<<\"EOF\"")
        && lines.last().is_some_and(|l| l.ends_with("EOF"))
    {
        lines = lines[1..lines.len() - 1].to_vec();
    }
    if lines.first().map(|l| l.trim()) != Some(BEGIN) {
        return Err("the patch doesn't start with *** Begin Patch");
    }
    if lines.last().map(|l| l.trim()) != Some(END) {
        return Err("the patch doesn't end with *** End Patch");
    }
    let mut files: Vec<FilePatch> = Vec::new();
    for line in &lines[1..lines.len() - 1] {
        // Inside an update a leading space makes a context line, not a header.
        let in_update =
            matches!(files.last(), Some(f) if matches!(f.change, Change::Update { .. }));
        let header = if in_update {
            line.trim_end()
        } else {
            line.trim()
        };
        if header == END {
            return Err("text after *** End Patch");
        }
        if let Some(path) = header.strip_prefix(ADD) {
            files.push(file(path, Change::Add(String::new())));
            continue;
        }
        if let Some(path) = header.strip_prefix(DELETE) {
            files.push(file(path, Change::Delete));
            continue;
        }
        if let Some(path) = header.strip_prefix(UPDATE) {
            files.push(file(
                path,
                Change::Update {
                    to: None,
                    chunks: Vec::new(),
                },
            ));
            continue;
        }
        match files.last_mut().map(|f| &mut f.change) {
            Some(Change::Add(text)) => {
                let added = line
                    .strip_prefix('+')
                    .ok_or("an added file's line doesn't start with +")?;
                text.push_str(added);
                text.push('\n');
            }
            Some(Change::Update { to, chunks }) => update_line(header, line, to, chunks)?,
            _ => return Err("a line outside any file"),
        }
    }
    for f in &files {
        if let Change::Update { chunks, .. } = &f.change {
            if chunks
                .last()
                .is_none_or(|c| c.old.is_empty() && c.new.is_empty())
            {
                return Err("an update with no lines");
            }
        }
    }
    Ok(files)
}

fn file(path: &str, change: Change) -> FilePatch {
    FilePatch {
        path: path.to_string(),
        change,
    }
}

fn update_line(
    trimmed: &str,
    line: &str,
    to: &mut Option<String>,
    chunks: &mut Vec<Chunk>,
) -> Result<(), &'static str> {
    let empty = |c: &Chunk| c.old.is_empty() && c.new.is_empty();
    if chunks.last().is_some_and(|c| c.at_end) && trimmed.is_empty() {
        return Ok(());
    }
    if chunks.is_empty() && to.is_none() {
        if let Some(path) = trimmed.strip_prefix(MOVE) {
            *to = Some(path.to_string());
            return Ok(());
        }
    }
    if trimmed == "@@" || trimmed.starts_with("@@ ") {
        if chunks.last().is_some_and(empty) {
            return Err("an @@ section with no lines");
        }
        chunks.push(Chunk {
            context: trimmed.strip_prefix("@@ ").map(String::from),
            ..Chunk::default()
        });
        return Ok(());
    }
    if chunks.last().is_some_and(|c| c.at_end) {
        return Err("a line after *** End of File that isn't an @@ section");
    }
    if trimmed == AT_END {
        match chunks.last_mut() {
            Some(c) if !empty(c) => c.at_end = true,
            _ => return Err("*** End of File with no lines before it"),
        }
        return Ok(());
    }
    if chunks.is_empty() {
        chunks.push(Chunk::default());
    }
    let chunk = chunks.last_mut().expect("a chunk was just pushed");
    if line.is_empty() {
        chunk.old.push(String::new());
        chunk.new.push(String::new());
    } else if let Some(kept) = line.strip_prefix(' ') {
        chunk.old.push(kept.to_string());
        chunk.new.push(kept.to_string());
    } else if let Some(added) = line.strip_prefix('+') {
        chunk.new.push(added.to_string());
    } else if let Some(removed) = line.strip_prefix('-') {
        chunk.old.push(removed.to_string());
    } else {
        return Err("an update line doesn't start with space, + or -");
    }
    Ok(())
}

/// The file's text after the chunks, as Codex writes it: lines found in
/// order, each more loosely if not exactly, and the result ends in `\n`.
pub fn apply(current: &str, chunks: &[Chunk]) -> Rebuilt {
    let mut lines: Vec<String> = current.split('\n').map(String::from).collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();
    let mut at = 0;
    for chunk in chunks {
        if let Some(context) = &chunk.context {
            match seek(&lines, std::slice::from_ref(context), at, false) {
                Some(i) => at = i + 1,
                None => return Rebuilt::Unknown("a patch's @@ line isn't in the file"),
            }
        }
        if chunk.old.is_empty() {
            replacements.push((lines.len(), 0, chunk.new.clone()));
            continue;
        }
        let (mut old, mut new) = (&chunk.old[..], &chunk.new[..]);
        let mut found = seek(&lines, old, at, chunk.at_end);
        if found.is_none() && old.last().is_some_and(String::is_empty) {
            old = &old[..old.len() - 1];
            if new.last().is_some_and(String::is_empty) {
                new = &new[..new.len() - 1];
            }
            found = seek(&lines, old, at, chunk.at_end);
        }
        let Some(start) = found else {
            return Rebuilt::Unknown("a patch's lines aren't in the file");
        };
        replacements.push((start, old.len(), new.to_vec()));
        at = start + old.len();
    }
    replacements.sort_by_key(|r| r.0);
    for (start, len, new) in replacements.into_iter().rev() {
        let end = (start + len).min(lines.len());
        lines.splice(start.min(end)..end, new);
    }
    if !lines.last().is_some_and(String::is_empty) {
        lines.push(String::new());
    }
    Rebuilt::Text(lines.join("\n"))
}

/// Where `pattern` starts at or after `start`: exactly, then ignoring
/// trailing space, then surrounding space, then typographic punctuation.
fn seek(lines: &[String], pattern: &[String], start: usize, at_end: bool) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }
    let last = lines.len() - pattern.len();
    let from = if at_end { last } else { start };
    let passes: [fn(&str) -> String; 4] = [
        |s| s.to_string(),
        |s| s.trim_end().to_string(),
        |s| s.trim().to_string(),
        plain,
    ];
    passes.iter().find_map(|norm| {
        (from..=last).find(|&i| {
            pattern
                .iter()
                .enumerate()
                .all(|(j, p)| norm(&lines[i + j]) == norm(p))
        })
    })
}

fn plain(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| match c {
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{2018}'..='\u{201B}' => '\'',
            '\u{201C}'..='\u{201F}' => '"',
            '\u{00A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(r: Rebuilt) -> String {
        match r {
            Rebuilt::Text(t) => t,
            Rebuilt::Unknown(why) => panic!("not rebuilt: {why}"),
        }
    }

    #[test]
    fn a_patch_names_each_file_and_what_happens_to_it() {
        let files = parse(
            "*** Begin Patch\n*** Add File: src/new.ts\n+export const a = 1;\n+\n*** Delete File: old.ts\n*** Update File: src/a.ts\n*** Move to: src/b.ts\n@@ function f() {\n-  return 1;\n+  return 2;\n*** End Patch\n",
        )
        .unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(
            files[0].change,
            Change::Add("export const a = 1;\n\n".into())
        );
        assert_eq!(files[1], file("old.ts", Change::Delete));
        let Change::Update { to, chunks } = &files[2].change else {
            panic!("not an update")
        };
        assert_eq!(to.as_deref(), Some("src/b.ts"));
        assert_eq!(chunks[0].context.as_deref(), Some("function f() {"));
        assert_eq!(chunks[0].old, ["  return 1;"]);
        assert_eq!(chunks[0].new, ["  return 2;"]);
    }

    #[test]
    fn a_heredoc_wrapped_patch_is_read_and_a_broken_one_is_refused() {
        let wrapped = "<<'EOF'\n*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch\nEOF";
        assert_eq!(
            parse(wrapped).unwrap()[0].change,
            Change::Add("hi\n".into())
        );
        assert!(parse("*** Begin Patch\n*** Add File: a\n+x\n").is_err());
        assert!(parse("*** Begin Patch\n*** Update File: a\n*** End Patch").is_err());
        assert!(parse("*** Begin Patch\n*** Add File: a\nno plus\n*** End Patch").is_err());
        assert!(parse("*** Begin Patch\n*** Update File: a\n@@\n@@\n+x\n*** End Patch").is_err());
    }

    #[test]
    fn chunks_apply_in_order_after_their_context_and_the_file_ends_in_a_newline() {
        let files = parse(
            "*** Begin Patch\n*** Update File: a.py\n@@ def g():\n-    return 1\n+    return 3\n@@\n x = 1\n+y = 2\n*** End Patch",
        )
        .unwrap();
        let Change::Update { chunks, .. } = &files[0].change else {
            panic!()
        };
        let before = "def f():\n    return 1\ndef g():\n    return 1\nx = 1";
        assert_eq!(
            text(apply(before, chunks)),
            "def f():\n    return 1\ndef g():\n    return 3\nx = 1\ny = 2\n"
        );
    }

    #[test]
    fn lines_match_loosely_as_codex_matches_them() {
        let chunk = Chunk {
            old: vec!["let s = \"a-b\";".into()],
            new: vec!["let s = \"c\";".into()],
            ..Chunk::default()
        };
        let before = "  let s = \u{201C}a\u{2013}b\u{201D};   \n";
        assert_eq!(text(apply(before, &[chunk])), "let s = \"c\";\n");
    }

    #[test]
    fn a_pure_addition_appends_and_end_of_file_matches_from_the_end() {
        let add = Chunk {
            new: vec!["tail".into()],
            ..Chunk::default()
        };
        assert_eq!(text(apply("a\nb\n", &[add])), "a\nb\ntail\n");
        let last = Chunk {
            old: vec!["x".into()],
            new: vec!["z".into()],
            at_end: true,
            ..Chunk::default()
        };
        assert_eq!(text(apply("x\ny\nx\n", &[last])), "x\ny\nz\n");
    }

    #[test]
    fn lines_that_are_not_there_are_unknown() {
        let chunk = Chunk {
            old: vec!["missing".into()],
            ..Chunk::default()
        };
        assert!(matches!(apply("a\n", &[chunk]), Rebuilt::Unknown(_)));
        let context = Chunk {
            context: Some("nowhere".into()),
            new: vec!["x".into()],
            ..Chunk::default()
        };
        assert!(matches!(apply("a\n", &[context]), Rebuilt::Unknown(_)));
    }
}
