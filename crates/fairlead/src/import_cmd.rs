//! `fairlead import` brings in what a team already keeps. `import lessons
//! FILE` makes one lesson file per `## ` heading of a lessons document: a
//! dry run unless `--write`, and the document itself is never written.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::config::Memory;
use regex::Regex;

use crate::knowledge::import::{self as doc, Known, Section};
use crate::knowledge::lesson::{self, Front, Source};

#[derive(clap::Subcommand)]
pub enum ImportAction {
    /// One lesson file per `## ` heading of a lessons document; a dry run unless --write.
    Lessons {
        /// The document, such as docs/LESSONS.md. It is only read.
        file: PathBuf,
        /// Write the lessons to `memory.dir`, never over a different file.
        #[arg(long)]
        write: bool,
        /// Print the lessons as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Turn path-scoped rule files (Claude Code `.md` with `paths`, Cursor `.mdc` with `globs` or `alwaysApply`) into skills plus `[[skills.routes]]`.
    Rules {
        /// The directory holding the rule files, such as `.claude/rules` or `.cursor/rules`.
        dir: PathBuf,
        /// Write the SKILL.md files and append the routes to fairlead.toml; without it, only print them.
        #[arg(long)]
        write: bool,
    },
}

pub fn run(action: ImportAction, cwd: &Path) -> ExitCode {
    let (file, write, json) = match action {
        ImportAction::Lessons { file, write, json } => (file, write, json),
        ImportAction::Rules { dir, write } => {
            return crate::import_rules_cmd::run(
                crate::import_rules_cmd::ImportAction::Rules { dir, write },
                cwd,
            )
        }
    };
    match lessons(&file, write, json, cwd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("import: {e}");
            ExitCode::from(2)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    New,
    Same,
    Differs,
    Written,
    Failed,
}

impl State {
    fn name(self, write: bool) -> &'static str {
        match self {
            State::New => "new",
            State::Same => "same",
            State::Differs if write => "refused",
            State::Differs => "differs",
            State::Written => "written",
            State::Failed => "failed",
        }
    }
}

/// A heading that became a lesson.
struct Made {
    section: Section,
    rel: String,
    front: Front,
    body: String,
    text: String,
    notes: Vec<String>,
    state: State,
}

/// A heading whose lesson failed the checks every lesson keeps.
struct Skipped {
    section: Section,
    reasons: Vec<String>,
}

struct Import {
    source: String,
    headings: usize,
    preamble: usize,
    outside: usize,
    made: Vec<Made>,
    skipped: Vec<Skipped>,
}

fn lessons(file: &Path, write: bool, json: bool, cwd: &Path) -> Result<ExitCode, String> {
    let (root, memory) = crate::lessons_cmd::memory_at(cwd)?;
    let abs = cwd.join(file);
    let text = std::fs::read_to_string(&abs).map_err(|e| format!("{}: {e}", file.display()))?;
    let canon_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    let inside = std::fs::canonicalize(&abs)
        .ok()
        .and_then(|a| fairlead_lang::tree::relative(&canon_root, &a));
    let source = inside
        .clone()
        .unwrap_or_else(|| file.display().to_string().replace('\\', "/"));
    let dates = inside
        .as_deref()
        .map(|rel| blame_dates(&root, rel))
        .unwrap_or_default();
    let known = Known::new(fairlead_lang::tree::Tree::scan(&root).files);
    let mut import = build(&source, &text, &known, &dates, &memory);
    for m in &mut import.made {
        let path = root.join(&m.rel);
        m.state = match std::fs::read_to_string(&path) {
            Ok(t) if t == m.text => State::Same,
            Ok(_) => State::Differs,
            Err(_) if path.exists() => State::Differs,
            Err(_) => State::New,
        };
        if write && m.state == State::New {
            let wrote = std::fs::create_dir_all(path.parent().expect("a lesson has a directory"))
                .and_then(|()| std::fs::write(&path, &m.text));
            m.state = match wrote {
                Ok(()) => State::Written,
                Err(e) => {
                    m.notes.push(format!("couldn't write it: {e}"));
                    State::Failed
                }
            };
        }
    }
    if json {
        print_json(&import, write);
    } else {
        print_text(&import, write, &memory);
    }
    let refused = import
        .made
        .iter()
        .any(|m| matches!(m.state, State::Differs | State::Failed));
    Ok(if import.skipped.is_empty() && !refused {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// The author date, in UTC, of each line of `rel`; empty when git can't say.
fn blame_dates(root: &Path, rel: &str) -> HashMap<usize, String> {
    let out = std::process::Command::new("git")
        .args(["blame", "--porcelain", "--", rel])
        .current_dir(root)
        .output();
    match out {
        Ok(o) if o.status.success() => parse_blame(&String::from_utf8_lossy(&o.stdout)),
        _ => HashMap::new(),
    }
}

/// `git blame --porcelain` gives a commit's headers only on its first line,
/// so each commit's author time is kept for the lines after.
fn parse_blame(text: &str) -> HashMap<usize, String> {
    let mut times: HashMap<&str, u64> = HashMap::new();
    let mut dates = HashMap::new();
    let (mut sha, mut line) = ("", 0usize);
    for row in text.lines() {
        if row.starts_with('\t') {
            if let Some(&t) = times.get(sha) {
                let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(t);
                dates.insert(
                    line,
                    fairlead_guard::events::timestamp(at)[..10].to_string(),
                );
            }
            continue;
        }
        if let Some(t) = row.strip_prefix("author-time ") {
            if let Ok(t) = t.trim().parse() {
                times.insert(sha, t);
            }
            continue;
        }
        let mut parts = row.split(' ');
        let first = parts.next().unwrap_or("");
        if first.len() >= 40 && first.bytes().all(|b| b.is_ascii_hexdigit()) {
            sha = first;
            line = parts.nth(1).and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    dates
}

/// The id a heading gets: its slug, made unique with `-2`, `-3`.
fn unique_id(section: &Section, used: &mut HashSet<String>) -> (String, Option<String>) {
    let base = match lesson::slug(&section.heading) {
        s if s.is_empty() => format!("heading-{}", section.line),
        s => s,
    };
    if used.insert(base.clone()) {
        return (base, None);
    }
    let id = (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !used.contains(id))
        .expect("some suffix is free");
    used.insert(id.clone());
    (id, Some(base))
}

fn build(
    source: &str,
    text: &str,
    known: &Known,
    dates: &HashMap<usize, String>,
    memory: &Memory,
) -> Import {
    let split = doc::split(text);
    let mut import = Import {
        source: source.to_string(),
        headings: split.sections.len(),
        preamble: split.preamble,
        outside: split.outside,
        made: Vec::new(),
        skipped: Vec::new(),
    };
    let mut used = HashSet::new();
    let today = lesson::today();
    for section in split.sections {
        let mut notes = Vec::new();
        let (id, clash) = unique_id(&section, &mut used);
        if let Some(base) = clash {
            notes.push(format!(
                "`{base}` is taken by an earlier heading, so this one is `{id}`"
            ));
        }
        if doc::names_only(&section.heading) {
            notes.push("the heading names it, so the title is the body's first sentence".into());
        }
        let paths = doc::paths(&section.heading, &section.body, known);
        if paths.is_empty() {
            notes.push("no path in the text; found only by search".into());
        }
        let full = format!("{source}#{}", section.anchor);
        let mut evidence = vec![full.clone()];
        evidence.extend(doc::links(&section.body));
        let (body, cut) = doc::capped(&section.body, memory.max_lines, &full);
        if cut {
            notes.push(format!(
                "the text is {} lines, over memory.max_lines ({}); the lesson links the rest",
                section.body.len(),
                memory.max_lines
            ));
        }
        let added = dates.get(&section.line).cloned().unwrap_or(today.clone());
        let front = Front {
            id,
            title: doc::title(&section.heading, &section.body),
            always: false,
            search: paths.is_empty(),
            paths,
            modules: Vec::new(),
            // No review date: one from the import day would differ on every
            // re-import, and one from blame would make an old document due at once.
            review_by: None,
            added,
            evidence,
            check: None,
            source: Source::Imported,
            confirmed_by: None,
        };
        let text = lesson::render(&front, &body);
        let rel = format!("{}/{}.md", memory.dir.trim_end_matches('/'), front.id);
        match lesson::parse(&rel, &text, memory) {
            Ok(_) => import.made.push(Made {
                section,
                rel,
                front,
                body,
                text,
                notes,
                state: State::New,
            }),
            Err(problems) => {
                let reasons = locate(&problems, &text, &section, source);
                import.skipped.push(Skipped { section, reasons });
            }
        }
    }
    import
}

/// A finding's line in the lesson, as the document's line where it has one.
fn locate(problems: &[String], text: &str, section: &Section, source: &str) -> Vec<String> {
    let at = Regex::new(r"^line (\d+): (.*)$").expect("the line pattern compiles");
    let front_lines = text
        .lines()
        .enumerate()
        .filter(|(_, l)| *l == "---")
        .nth(1)
        .map_or(0, |(i, _)| i + 1);
    problems
        .iter()
        .map(|p| match at.captures(p) {
            Some(c) => {
                let n: usize = c[1].parse().unwrap_or(0);
                if n > front_lines {
                    format!(
                        "{source}:{}: {}",
                        section.body_line + n - front_lines - 1,
                        &c[2]
                    )
                } else {
                    format!("the heading or its links: {}", &c[2])
                }
            }
            None => p.clone(),
        })
        .collect()
}

fn reason_kind(reasons: &[String]) -> &'static str {
    if reasons.iter().any(|r| r.contains("looks like")) {
        "a secret or personal data"
    } else if reasons.iter().any(|r| r.contains("the body is empty")) {
        "no text under the heading"
    } else {
        "a failed lesson check"
    }
}

fn summary(import: &Import) -> String {
    let with_paths = import.made.iter().filter(|m| !m.front.search).count();
    let mut kinds: Vec<&str> = Vec::new();
    for s in &import.skipped {
        let k = reason_kind(&s.reasons);
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let why = if kinds.is_empty() {
        String::new()
    } else {
        format!(" ({})", kinds.join(", "))
    };
    format!(
        "import: {}: {} headings → {} lessons, {with_paths} with paths, {} found only by search, {} skipped{why}",
        import.source,
        import.headings,
        import.made.len(),
        import.made.len() - with_paths,
        import.skipped.len()
    )
}

fn print_text(import: &Import, write: bool, memory: &Memory) {
    println!("{}", summary(import));
    if import.preamble > 0 {
        println!(
            "preamble: {} lines before the first `## ` heading, skipped",
            import.preamble
        );
    }
    if import.outside > 0 {
        println!(
            "outside: {} lines under a later `# ` heading, which no `## ` heading holds, skipped",
            import.outside
        );
    }
    for m in &import.made {
        let scope = if m.front.search {
            "search".to_string()
        } else {
            m.front.paths.join(", ")
        };
        println!("{}  [{scope}]  {}", m.front.id, m.front.title);
        for n in &m.notes {
            println!("    {n}");
        }
        match m.state {
            State::Same => println!("    {} is already there, the same", m.rel),
            State::Differs if write => {
                println!("    refused: {} is already there and differs", m.rel)
            }
            State::Differs => println!(
                "    {} is already there and differs; --write leaves it",
                m.rel
            ),
            _ => {}
        }
    }
    for s in &import.skipped {
        println!(
            "skipped  ## {} (line {}): {}",
            s.section.heading,
            s.section.line,
            s.reasons.join("; ")
        );
    }
    let count = |state: State| import.made.iter().filter(|m| m.state == state).count();
    if write {
        println!(
            "wrote {} lessons to {}; {} already there and the same, {} refused; {} is unchanged",
            count(State::Written),
            memory.dir,
            count(State::Same),
            count(State::Differs) + count(State::Failed),
            import.source
        );
    } else {
        println!(
            "dry run: nothing written; --write writes {} new lessons to {}",
            count(State::New),
            memory.dir
        );
    }
}

fn print_json(import: &Import, write: bool) {
    let lessons: Vec<serde_json::Value> = import
        .made
        .iter()
        .map(|m| {
            serde_json::json!({
                "path": m.rel,
                "status": m.state.name(write),
                "heading": m.section.heading,
                "line": m.section.line,
                "notes": m.notes,
                "lesson": m.front,
                "body": m.body,
            })
        })
        .collect();
    let skipped: Vec<serde_json::Value> = import
        .skipped
        .iter()
        .map(|s| {
            serde_json::json!({
                "heading": s.section.heading,
                "line": s.section.line,
                "reasons": s.reasons,
            })
        })
        .collect();
    let out = serde_json::json!({
        "source": import.source,
        "written": write,
        "headings": import.headings,
        "preamble_lines": import.preamble,
        "outside_lines": import.outside,
        "lessons": lessons,
        "skipped": skipped,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&out).expect("the import serializes")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blame_dates_carry_to_every_line_of_a_commit() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let text = format!(
            "{a} 1 1 2\nauthor-time 86400\nsummary x\n\tline one\n{a} 2 2\n\tline two\n{b} 3 3 1\nauthor-time 0\n\tline three\n"
        );
        let dates = parse_blame(&text);
        assert_eq!(dates[&1], "1970-01-02");
        assert_eq!(dates[&2], "1970-01-02");
        assert_eq!(dates[&3], "1970-01-01");
    }

    #[test]
    fn repeated_headings_get_suffixed_ids_and_every_heading_is_accounted_for() {
        let doc = "Intro.\n\n## Same\nOne.\n\n## Same\nTwo.\n\n## Same\nThree.\n\n## Empty\n";
        let known = Known::new(Vec::new());
        let import = build(
            "docs/l.md",
            doc,
            &known,
            &HashMap::new(),
            &Memory::default(),
        );
        let ids: Vec<&str> = import.made.iter().map(|m| m.front.id.as_str()).collect();
        assert_eq!(ids, ["same", "same-2", "same-3"]);
        assert_eq!(import.made[1].front.evidence, ["docs/l.md#same-1"]);
        assert_eq!(import.skipped.len(), 1);
        assert_eq!(import.made.len() + import.skipped.len(), import.headings);
        assert!(summary(&import).contains(
            "4 headings → 3 lessons, 0 with paths, 3 found only by search, 1 skipped (no text under the heading)"
        ));
    }

    #[test]
    fn a_secret_is_reported_at_its_line_in_the_document() {
        let doc = "## Keys\nFine.\nkey AKIAABCDEFGHIJKLMNOP here\n";
        let import = build(
            "docs/l.md",
            doc,
            &Known::new(Vec::new()),
            &HashMap::new(),
            &Memory::default(),
        );
        let reasons = import.skipped[0].reasons.join("\n");
        assert!(
            reasons.contains("docs/l.md:3: looks like a cloud access key"),
            "{reasons}"
        );
    }
}
