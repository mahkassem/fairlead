//! What `fairlead find` searches, gathered fresh on every run: lessons,
//! routed skills, the headings of the repository's Markdown, and the names
//! the PHP and JVM scanners saw declared. Each entry carries the file it
//! lives in, and lessons and skills their scope, for nearness to a brief.

use std::collections::{HashMap, VecDeque};

use fairlead_lang::graph::Graph;
use fairlead_lang::Scan;
use serde::Serialize;

use super::lesson::Lesson;
use super::route::Scope;
use super::skill::Skill;

/// Directories whose Markdown is someone else's or a build's output.
const SKIPPED: [&str; 6] = ["node_modules", "vendor", ".git", "target", "dist", "build"];
/// How far nearness to a brief reaches, in import edges either way.
pub const MAX_HOPS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Lesson,
    Skill,
    Doc,
    Symbol,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Lesson => "lesson",
            Kind::Skill => "skill",
            Kind::Doc => "doc",
            Kind::Symbol => "symbol",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    /// A lesson's title, a skill's name, a heading's text or a qualified name.
    pub name: String,
    /// Where to look: a file, or a docs file with its `#anchor`.
    pub path: String,
    /// The file `path` is in, for its distance from a brief.
    pub file: String,
    /// A lesson's or a skill's scope: its nearness is that of the nearest
    /// file the scope covers.
    pub scope: Option<Scope>,
    /// Weighed double.
    pub title: String,
    pub body: String,
    pub snippet: String,
}

pub fn lessons(lessons: Vec<Lesson>) -> impl Iterator<Item = Entry> {
    lessons.into_iter().map(|l| Entry {
        kind: Kind::Lesson,
        name: l.front.title.clone(),
        snippet: super::short(&first_paragraph(&l.body), 160),
        body: format!("{} {}", l.front.id, l.body),
        title: l.front.title,
        file: l.path.clone(),
        path: l.path,
        scope: Some(l.scope),
    })
}

pub fn skills(skills: Vec<Skill>) -> impl Iterator<Item = Entry> {
    skills.into_iter().map(|s| Entry {
        kind: Kind::Skill,
        snippet: super::short(&s.description, 160),
        title: s.name.clone(),
        name: s.name,
        body: s.description,
        file: s.path.clone(),
        path: s.path,
        scope: Some(s.scope),
    })
}

/// A heading's first paragraph, or the first lines of a lesson's body.
fn first_paragraph(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .skip_while(|l| l.is_empty())
        .take_while(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every `#` to `####` heading of the tree's Markdown files, with its
/// section's first paragraph; `skip` is a directory left out, such as the
/// lessons' own.
pub fn docs(scan: &Scan, skip: &str) -> Vec<Entry> {
    let skip = format!("{}/", skip.trim_end_matches('/'));
    let mut out = Vec::new();
    for file in scan.tree.files.iter().filter(|f| {
        f.ends_with(".md") && !f.starts_with(&skip) && !f.split('/').any(|c| SKIPPED.contains(&c))
    }) {
        let Ok(text) = std::fs::read_to_string(scan.tree.abs(file)) else {
            continue;
        };
        for h in headings(&text) {
            out.push(Entry {
                kind: Kind::Doc,
                path: format!("{file}#{}", h.anchor),
                file: file.clone(),
                scope: None,
                snippet: super::short(&h.paragraph, 160),
                title: h.text.clone(),
                name: h.text,
                body: h.paragraph,
            });
        }
    }
    out
}

/// The declarations the PHP and JVM indexes hold; they keep no line, so
/// the path is the file.
pub fn symbols(scan: &Scan) -> Vec<Entry> {
    let php = scan
        .autoload
        .declarations()
        .iter()
        .map(|(n, f)| (n.as_str(), f.as_str()));
    php.chain(scan.jvm.declarations())
        .map(|(name, file)| Entry {
            kind: Kind::Symbol,
            name: name.to_string(),
            path: file.to_string(),
            file: file.to_string(),
            scope: None,
            title: name.to_string(),
            body: String::new(),
            snippet: name.to_string(),
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub struct Heading {
    pub text: String,
    pub anchor: String,
    pub paragraph: String,
}

/// ATX headings one to four deep outside code fences and front matter,
/// each with the paragraph that opens its section, if one does.
pub fn headings(text: &str) -> Vec<Heading> {
    let lines: Vec<&str> = text.lines().collect();
    let mut at = 0;
    if lines.first().is_some_and(|l| l.trim_end() == "---") {
        at = lines[1..]
            .iter()
            .position(|l| l.trim_end() == "---")
            .map_or(0, |end| end + 2);
    }
    let mut out = Vec::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut fence: Option<char> = None;
    while at < lines.len() {
        let line = lines[at];
        at += 1;
        if let Some(mark) = fence_mark(line) {
            match fence {
                None => fence = Some(mark),
                Some(open) if open == mark => fence = None,
                Some(_) => {}
            }
            continue;
        }
        let Some(text) = fence.is_none().then(|| heading(line)).flatten() else {
            continue;
        };
        let paragraph = lines[at..]
            .iter()
            .map(|l| l.trim())
            .skip_while(|l| l.is_empty())
            .take_while(|l| !l.is_empty() && !l.starts_with('#') && fence_mark(l).is_none())
            .collect::<Vec<_>>()
            .join(" ");
        let base = slug(&text);
        let n = used.entry(base.clone()).or_insert(0);
        let anchor = if *n == 0 { base } else { format!("{base}-{n}") };
        *n += 1;
        out.push(Heading {
            text,
            anchor,
            paragraph,
        });
    }
    out
}

fn fence_mark(line: &str) -> Option<char> {
    let t = line.trim_start();
    if t.starts_with("```") {
        Some('`')
    } else if t.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

/// The text of a `#` to `####` heading line.
fn heading(line: &str) -> Option<String> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    let rest = &line[hashes..];
    if !(1..=4).contains(&hashes) || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A heading's anchor as the book and code hosts make it: lowercase, words
/// joined by `-`, punctuation dropped.
pub fn slug(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c.to_lowercase().next().unwrap_or(c))
            } else if c.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// Each file within `MAX_HOPS` import edges of `starts`, either way, with
/// its distance; a start counts as 0 even when the graph doesn't have it.
/// The walk doesn't go past a barrier, as the router's doesn't.
pub fn distances(graph: &Graph, starts: &[String]) -> HashMap<String, usize> {
    let mut out: HashMap<String, usize> = starts.iter().map(|s| (s.clone(), 0)).collect();
    let mut seen = vec![false; graph.files.len()];
    let mut queue: VecDeque<(u32, usize)> = VecDeque::new();
    for id in starts.iter().filter_map(|s| graph.id(s)) {
        if !seen[id as usize] {
            seen[id as usize] = true;
            queue.push_back((id, 0));
        }
    }
    while let Some((file, depth)) = queue.pop_front() {
        if depth == MAX_HOPS || (depth > 0 && graph.is_barrier(file)) {
            continue;
        }
        let forward = graph.dependencies(file).iter().map(|&(d, _)| d);
        let back = graph.importers(file).into_iter().map(|(i, _)| i);
        for other in forward.chain(back).collect::<Vec<_>>() {
            if std::mem::replace(&mut seen[other as usize], true) {
                continue;
            }
            out.insert(graph.files[other as usize].clone(), depth + 1);
            queue.push_back((other, depth + 1));
        }
    }
    out
}

/// An entry's distance from the brief: its file's, or for a scoped entry
/// the nearest file its scope covers. `near` is the brief's reach, nearest
/// first. `always` alone covers no file, so it brings no boost.
pub fn distance(
    entry: &Entry,
    near: &[(String, usize, Option<String>)],
    by_file: &HashMap<String, usize>,
) -> Option<usize> {
    match &entry.scope {
        Some(scope) => near
            .iter()
            .find(|(p, _, m)| scope.covers(p, m.as_deref()))
            .map(|(_, d, _)| *d),
        None => by_file.get(&entry.file).copied(),
    }
}

/// The score multiplier for a distance: 2 on a brief's own path, 1.5 one
/// hop away, down to 1.25 at three, and 1 (no boost) beyond or with no brief.
pub fn boost(distance: Option<usize>) -> f64 {
    distance.map_or(1.0, |d| 1.0 + 1.0 / (1.0 + d as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fairlead_lang::graph::EdgeKind;

    #[test]
    fn headings_skip_fences_and_front_matter_and_carry_their_first_paragraph() {
        let text = "---\ntitle: x\n# not a heading\n---\n# Retry budget\n\nEach call\nretries twice.\n\nMore.\n```sh\n# a comment\n```\n## Retry budget\n##### Too deep\n#hashtag\n### `fairlead find` (search)\n";
        let found = headings(text);
        let names: Vec<(&str, &str)> = found
            .iter()
            .map(|h| (h.text.as_str(), h.anchor.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("Retry budget", "retry-budget"),
                ("Retry budget", "retry-budget-1"),
                ("`fairlead find` (search)", "fairlead-find-search"),
            ]
        );
        assert_eq!(found[0].paragraph, "Each call retries twice.");
        assert_eq!(found[1].paragraph, "");
    }

    /// a.ts imports b.ts, which imports c.ts, which imports d.ts, which
    /// imports e.ts.
    fn chain() -> Graph {
        let files: Vec<String> = ["a.ts", "b.ts", "c.ts", "d.ts", "e.ts"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut g = Graph::with_files(files, Vec::new());
        for i in 0..4 {
            g.add_edge(i, i + 1, EdgeKind::Import);
        }
        g
    }

    #[test]
    fn distance_goes_both_ways_and_stops_at_three_hops() {
        let d = distances(&chain(), &["c.ts".into(), "new.ts".into()]);
        assert_eq!(d["c.ts"], 0);
        assert_eq!(d["new.ts"], 0);
        assert_eq!((d["b.ts"], d["d.ts"], d["a.ts"], d["e.ts"]), (1, 1, 2, 2));
        let d = distances(&chain(), &["a.ts".into()]);
        assert_eq!(d["d.ts"], 3);
        assert!(!d.contains_key("e.ts"));
    }

    #[test]
    fn a_hit_nearer_the_brief_outranks_an_equal_hit_farther_away() {
        let by_file = distances(&chain(), &["a.ts".into()]);
        let mut near: Vec<(String, usize, Option<String>)> =
            by_file.iter().map(|(f, d)| (f.clone(), *d, None)).collect();
        near.sort_by(|x, y| (x.1, &x.0).cmp(&(y.1, &y.0)));
        let entry = |file: &str, scope: Option<Scope>| Entry {
            kind: Kind::Lesson,
            name: file.into(),
            path: file.into(),
            file: file.into(),
            scope,
            title: String::new(),
            body: String::new(),
            snippet: String::new(),
        };
        let scoped = |globs: &[&str]| {
            let globs: Vec<String> = globs.iter().map(|g| g.to_string()).collect();
            Some(Scope::new(&globs, &[], false).unwrap())
        };
        let (bm25_score, near_one, far_one) = (3.0, entry("b.ts", None), entry("d.ts", None));
        let near_score = bm25_score * boost(distance(&near_one, &near, &by_file));
        let far_score = bm25_score * boost(distance(&far_one, &near, &by_file));
        assert!(near_score > far_score, "{near_score} vs {far_score}");
        assert_eq!(boost(distance(&entry("e.ts", None), &near, &by_file)), 1.0);
        // A lesson scoped to c.ts and e.ts is two hops away, through c.ts.
        let lesson = entry(".fairlead/lessons/x.md", scoped(&["c.ts", "e.ts"]));
        assert_eq!(distance(&lesson, &near, &by_file), Some(2));
        let always = Scope::new(&[], &[], true).unwrap();
        assert_eq!(
            distance(&entry("y.md", Some(always)), &near, &by_file),
            None
        );
        assert_eq!(boost(None), 1.0);
        assert_eq!(boost(Some(0)), 2.0);
    }
}
