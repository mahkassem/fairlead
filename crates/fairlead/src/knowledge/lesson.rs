//! A lesson: one small committed file with a scope, its evidence and where
//! it came from, offered for a change the way a test is picked.

use std::path::{Path, PathBuf};

use fairlead_core::config::Memory;
use fairlead_core::pattern::Pattern;
use serde::{Deserialize, Serialize};

use super::route::Scope;
use super::secrets;

/// Where a lesson came from. Something learned has to be confirmed by a
/// person or taught by a mistake anyone can see; a guess is neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// A person confirmed it; `confirmed_by` names them.
    Person,
    /// An obvious mistake taught it: the evidence is the failure.
    Mistake,
    /// Brought in from a document the team already kept.
    Imported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Front {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub always: bool,
    pub added: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_by: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    /// The rule or test that enforces it, which makes it cheap to trust.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_by: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Lesson {
    pub front: Front,
    pub body: String,
    /// From the repository root.
    pub path: String,
    pub scope: Scope,
}

impl Lesson {
    pub fn due(&self, today: &str) -> bool {
        self.front.review_by.as_deref().is_some_and(|d| d < today)
    }
}

/// A lesson file that can't be offered, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bad {
    pub path: String,
    pub reason: String,
}

/// Splits `---` front matter from the body.
fn split(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    let front = &rest[..end + 1];
    let after = &rest[end + 4..];
    let body = after
        .strip_prefix("\r\n")
        .or_else(|| after.strip_prefix('\n'))
        .unwrap_or(after);
    Some((front, body))
}

/// Parses and checks one lesson file's text; every problem, not just the first.
pub fn parse(path: &str, text: &str, memory: &Memory) -> Result<Lesson, Vec<String>> {
    let Some((front, body)) = split(text) else {
        return Err(vec!["no `---` front matter".into()]);
    };
    let front: Front =
        serde_saphyr::from_str(front).map_err(|e| vec![first_line(&e.to_string())])?;
    let mut problems = check(&front, body, memory);
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !front.id.is_empty() && stem != front.id {
        problems.push(format!(
            "the file is named `{stem}` but its id is `{}`",
            front.id
        ));
    }
    for finding in secrets::scan(text) {
        problems.push(finding.to_string());
    }
    let scope = Scope::new(&front.paths, &front.modules, front.always);
    match scope {
        Ok(scope) if problems.is_empty() => Ok(Lesson {
            front,
            body: body.trim_end().to_string(),
            path: path.to_string(),
            scope,
        }),
        Ok(_) => Err(problems),
        Err(e) => {
            problems.push(e);
            Err(problems)
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

/// The rules every lesson keeps, whoever wrote it.
pub fn check(front: &Front, body: &str, memory: &Memory) -> Vec<String> {
    let mut problems = Vec::new();
    if !is_slug(&front.id) {
        problems.push("`id` must be lowercase letters, digits and dashes".into());
    }
    if front.title.trim().is_empty() || front.title.contains('\n') {
        problems.push("`title` must be one line".into());
    }
    if front.paths.is_empty() && front.modules.is_empty() && !front.always {
        problems.push("a scope is needed: `paths`, `modules` or `always: true`".into());
    }
    for p in &front.paths {
        if let Err(e) = Pattern::new(p) {
            problems.push(format!("`paths` entry `{p}`: {e}"));
        }
    }
    for (key, date) in [
        ("added", Some(&front.added)),
        ("review_by", front.review_by.as_ref()),
    ] {
        if let Some(d) = date {
            if days(d).is_none() {
                problems.push(format!("`{key}` must be a date like 2026-01-31"));
            }
        }
    }
    if front.evidence.iter().all(|e| e.trim().is_empty()) {
        problems.push("at least one `evidence` link is needed".into());
    }
    if front.source == Source::Person
        && front
            .confirmed_by
            .as_deref()
            .is_none_or(|c| c.trim().is_empty())
    {
        problems.push("`source: person` needs `confirmed_by`, the person who confirmed it".into());
    }
    let lines = body.trim_end().lines().count();
    if body.trim().is_empty() {
        problems.push("the body is empty".into());
    } else if lines > memory.max_lines {
        problems.push(format!(
            "the body is {lines} lines, over `memory.max_lines` ({}); link a doc instead",
            memory.max_lines
        ));
    }
    problems
}

fn is_slug(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A title as an id: lowercase words joined by dashes, at most 60 characters.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for word in title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if out.len() + word.len() + 1 > 60 {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word.to_ascii_lowercase());
    }
    out
}

/// Every lesson under `memory.dir`, and the files that couldn't be read as one.
pub fn load(root: &Path, memory: &Memory) -> (Vec<Lesson>, Vec<Bad>) {
    let dir = root.join(&memory.dir);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| {
            r.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut lessons: Vec<Lesson> = Vec::new();
    let mut bad = Vec::new();
    for file in files {
        let rel = format!(
            "{}/{}",
            memory.dir.trim_end_matches('/'),
            file.file_name().and_then(|n| n.to_str()).unwrap_or("")
        );
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                bad.push(Bad {
                    path: rel,
                    reason: e.to_string(),
                });
                continue;
            }
        };
        match parse(&rel, &text, memory) {
            Ok(l) if lessons.iter().any(|o| o.front.id == l.front.id) => bad.push(Bad {
                path: rel,
                reason: format!("the id `{}` is used twice", l.front.id),
            }),
            Ok(l) => lessons.push(l),
            Err(problems) => bad.push(Bad {
                path: rel,
                reason: problems.join("; "),
            }),
        }
    }
    (lessons, bad)
}

/// The file `learn` writes: front matter in a fixed order, then the body.
pub fn render(front: &Front, body: &str) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("a string serializes");
    let list = |items: &[String]| {
        let items: Vec<String> = items.iter().map(|i| q(i)).collect();
        format!("[{}]", items.join(", "))
    };
    let mut out = String::from("---\n");
    out.push_str(&format!("id: {}\n", front.id));
    out.push_str(&format!("title: {}\n", q(&front.title)));
    if !front.paths.is_empty() {
        out.push_str(&format!("paths: {}\n", list(&front.paths)));
    }
    if !front.modules.is_empty() {
        out.push_str(&format!("modules: {}\n", list(&front.modules)));
    }
    if front.always {
        out.push_str("always: true\n");
    }
    out.push_str(&format!("added: {}\n", front.added));
    if let Some(r) = &front.review_by {
        out.push_str(&format!("review_by: {r}\n"));
    }
    out.push_str(&format!("evidence: {}\n", list(&front.evidence)));
    if let Some(c) = &front.check {
        out.push_str(&format!("check: {}\n", q(c)));
    }
    let source = match front.source {
        Source::Person => "person",
        Source::Mistake => "mistake",
        Source::Imported => "imported",
    };
    out.push_str(&format!("source: {source}\n"));
    if let Some(c) = &front.confirmed_by {
        out.push_str(&format!("confirmed_by: {}\n", q(c)));
    }
    out.push_str("---\n");
    out.push_str(body.trim_end());
    out.push('\n');
    out
}

/// Today in UTC, as `YYYY-MM-DD`.
pub fn today() -> String {
    fairlead_guard::events::timestamp(std::time::SystemTime::now())[..10].to_string()
}

/// Days since 1970-01-01 for a `YYYY-MM-DD` date.
pub fn days(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `date` plus `n` days.
pub fn add_days(date: &str, n: i64) -> Option<String> {
    let secs = u64::try_from(days(date)? + n).ok()? * 86_400;
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    Some(fairlead_guard::events::timestamp(at)[..10].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Memory {
        Memory::default()
    }

    const GOOD: &str = "---\nid: settle-at-sign-off\ntitle: \"Leave settles at sign-off\"\npaths: [\"src/leave/**\"]\nadded: 2026-09-12\nreview_by: 2026-12-11\nevidence: [\"https://example.com/pr/1\"]\nsource: person\nconfirmed_by: \"a-reviewer\"\n---\nA decision that costs money happens at the final stage.\n";

    #[test]
    fn a_complete_lesson_parses_and_renders_back_the_same() {
        let l = parse(".fairlead/lessons/settle-at-sign-off.md", GOOD, &memory()).unwrap();
        assert_eq!(l.front.id, "settle-at-sign-off");
        assert!(l.scope.paths[0].is_match("src/leave/a.ts"));
        assert_eq!(render(&l.front, &l.body), GOOD);
    }

    #[test]
    fn each_missing_piece_is_named() {
        let text = "---\nid: x\ntitle: T\nadded: 2026-09-12\nsource: person\n---\nbody\n";
        let problems = parse(".fairlead/lessons/x.md", text, &memory()).unwrap_err();
        let all = problems.join("\n");
        assert!(all.contains("a scope is needed"), "{all}");
        assert!(all.contains("evidence"), "{all}");
        assert!(all.contains("confirmed_by"), "{all}");
    }

    #[test]
    fn a_long_body_a_wrong_file_name_and_a_secret_are_refused() {
        let long = format!(
            "{}{}",
            GOOD.replace("person\nconfirmed_by: \"a-reviewer\"", "mistake"),
            "x\n".repeat(12)
        );
        let all = parse(".fairlead/lessons/other.md", &long, &memory())
            .unwrap_err()
            .join("\n");
        assert!(all.contains("over `memory.max_lines`"), "{all}");
        assert!(all.contains("named `other`"), "{all}");
        let leaked = GOOD.replace("final stage.", "final stage. key AKIAABCDEFGHIJKLMNOP");
        let all = parse(
            ".fairlead/lessons/settle-at-sign-off.md",
            &leaked,
            &memory(),
        )
        .unwrap_err()
        .join("\n");
        assert!(all.contains("access key"), "{all}");
        assert!(
            !all.contains("AKIAABCDEFGHIJKLMNOP"),
            "never echoes it: {all}"
        );
    }

    #[test]
    fn dates_round_trip_and_due_is_past_review_by() {
        assert_eq!(days("1970-01-01"), Some(0));
        assert_eq!(add_days("2026-12-31", 1).as_deref(), Some("2027-01-01"));
        assert_eq!(add_days("2024-02-28", 1).as_deref(), Some("2024-02-29"));
        assert_eq!(days("2026-13-01"), None);
        let l = parse(".fairlead/lessons/settle-at-sign-off.md", GOOD, &memory()).unwrap();
        assert!(l.due("2026-12-12"));
        assert!(!l.due("2026-12-11"));
    }

    #[test]
    fn titles_become_ids() {
        assert_eq!(
            slug("Leave settles at sign-off!"),
            "leave-settles-at-sign-off"
        );
        assert!(slug(&"word ".repeat(40)).len() <= 60);
    }
}
