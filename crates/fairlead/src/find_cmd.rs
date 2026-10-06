//! `fairlead find <query>`: one search over what the project knows, its
//! lessons, routed skills, docs headings and declared names, ranked by
//! BM25 and lifted by nearness to the session's brief.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use fairlead_core::config::LoadOptions;
use serde::Serialize;

use crate::brief_cmd::{Store, SESSION_ENV};
use crate::knowledge::bm25::Index;
use crate::knowledge::corpus::{self, Entry, Kind};
use crate::knowledge::{lesson, short, skill};

#[derive(clap::Args)]
pub struct FindArgs {
    /// The words to look for; quotes aren't needed.
    #[arg(required_unless_present = "symbol", num_args = 1..)]
    query: Vec<String>,
    /// Where NAME is declared (PHP, Java and Kotlin), by its exact short or
    /// qualified name: case-sensitive matches, or else case-insensitive ones.
    #[arg(long, value_name = "NAME", conflicts_with = "query")]
    symbol: Option<String>,
    /// How many hits to show, at most 50.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u8).range(1..=50))]
    limit: u8,
    /// The agent session whose brief ranks the hits, over `CLAUDE_CODE_SESSION_ID`.
    #[arg(long)]
    session: Option<String>,
    /// Print the hits as JSON.
    #[arg(long)]
    json: bool,
    /// Override a config value for this run.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    sets: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Hit {
    pub kind: Kind,
    pub name: String,
    pub path: String,
    pub score: f64,
    pub snippet: String,
}

#[derive(Serialize)]
struct Answer<'a> {
    query: &'a str,
    /// The brief whose paths ranked the hits, when there was one.
    brief: Option<String>,
    hits: Vec<Hit>,
}

pub fn run(args: FindArgs, cwd: &Path) -> ExitCode {
    match find(&args, cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("find: {e}");
            ExitCode::from(2)
        }
    }
}

fn find(args: &FindArgs, cwd: &Path) -> Result<(), String> {
    let loaded = fairlead_core::config::load(cwd, &LoadOptions::from_process(args.sets.clone()))
        .map_err(|e| e.to_string())?;
    if !loaded.problems.is_empty() {
        eprintln!("find: the config has problems (see `fairlead config check`); searching anyway");
    }
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    let config = loaded.config;
    let scan = fairlead_lang::build(&root, &config)
        .map_err(|e| format!("could not read {}: {e}", root.display()))?;
    let (lessons, bad_lessons) = lesson::load(&root, &config.memory);
    let (skills, bad_skills) = skill::load(&root, &config.skills);
    let unread = bad_lessons.len() + bad_skills.len();
    if unread > 0 {
        eprintln!("find: {unread} lesson or skill file(s) can't be read and aren't searched; `fairlead lessons check` names them");
    }
    let mut entries: Vec<Entry> = corpus::lessons(lessons)
        .chain(corpus::skills(skills))
        .collect();
    entries.extend(corpus::docs(&scan, &config.memory.dir));
    entries.extend(corpus::symbols(&scan));

    let session = args
        .session
        .clone()
        .or_else(|| std::env::var(SESSION_ENV).ok());
    let brief = Store::open(&root).and_then(|s| s.current(session.as_deref()));
    let modules =
        fairlead_tests::modules::Modules::discover(&scan.tree, &scan.packages, &config.modules)
            .unwrap_or_default();
    let by_file = brief
        .as_ref()
        .map_or_else(HashMap::new, |b| corpus::distances(&scan.graph, &b.paths));
    let mut near: Vec<(String, usize, Option<String>)> = by_file
        .iter()
        .map(|(f, d)| (f.clone(), *d, modules.name_of(f).map(str::to_string)))
        .collect();
    near.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    let nearness = |e: &Entry| corpus::boost(corpus::distance(e, &near, &by_file));

    let (query, scored) = match &args.symbol {
        Some(name) => (
            name.clone(),
            symbol_matches(&entries, name)
                .into_iter()
                .map(|i| (i, nearness(&entries[i])))
                .collect(),
        ),
        None => {
            let query = args.query.join(" ");
            let index = Index::new(entries.iter().map(|e| (e.title.as_str(), e.body.as_str())));
            let scored: Vec<(usize, f64)> = index
                .search(&query)
                .into_iter()
                .map(|(i, s)| (i, s * nearness(&entries[i])))
                .collect();
            (query, scored)
        }
    };
    let hits = rank(&entries, scored, usize::from(args.limit));
    let answer = Answer {
        query: &query,
        brief: brief.map(|b| b.id),
        hits,
    };
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&answer).expect("hits serialize")
        );
    } else {
        print!("{}", text(&answer));
    }
    Ok(())
}

/// The declarations named `name`, as written or by their last part
/// (`User` for `App\Models\User`): the case-sensitive ones, or when there
/// are none, the ones that match ignoring case.
pub fn symbol_matches(entries: &[Entry], name: &str) -> Vec<usize> {
    let name = name.trim_start_matches('\\');
    let last = |n: &str| n.rsplit(['\\', '.']).next().unwrap_or(n).to_string();
    let find = |same: &dyn Fn(&str, &str) -> bool| -> Vec<usize> {
        entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == Kind::Symbol)
            .filter(|(_, e)| same(&e.name, name) || same(&last(&e.name), name))
            .map(|(i, _)| i)
            .collect()
    };
    let exact = find(&|a, b| a == b);
    if exact.is_empty() {
        find(&|a, b| a.to_lowercase() == b.to_lowercase())
    } else {
        exact
    }
}

/// Best first, at most `limit`; ties go by kind, then name, then path.
fn rank(entries: &[Entry], mut scored: Vec<(usize, f64)>, limit: usize) -> Vec<Hit> {
    scored.sort_by(|(a, sa), (b, sb)| {
        let (ea, eb) = (&entries[*a], &entries[*b]);
        sb.total_cmp(sa)
            .then_with(|| (ea.kind, &ea.name, &ea.path).cmp(&(eb.kind, &eb.name, &eb.path)))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(i, score)| {
            let e = &entries[i];
            Hit {
                kind: e.kind,
                name: e.name.clone(),
                path: e.path.clone(),
                score: (score * 1000.0).round() / 1000.0,
                snippet: e.snippet.clone(),
            }
        })
        .collect()
}

fn text(answer: &Answer) -> String {
    if answer.hits.is_empty() {
        return format!("nothing matches `{}`\n", answer.query);
    }
    let names: Vec<String> = answer.hits.iter().map(|h| short(&h.name, 60)).collect();
    let width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
    let path_width = answer
        .hits
        .iter()
        .map(|h| h.path.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (hit, name) in answer.hits.iter().zip(&names) {
        out.push_str(&format!(
            "{:<6}  {name:<width$}  {:<path_width$}  {:.2}\n",
            hit.kind.as_str(),
            hit.path,
            hit.score
        ));
    }
    if let Some(id) = &answer.brief {
        out.push_str(&format!("hits near brief {id}'s paths are boosted\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(name: &str, file: &str) -> Entry {
        Entry {
            kind: Kind::Symbol,
            name: name.into(),
            path: file.into(),
            file: file.into(),
            scope: None,
            title: name.into(),
            body: String::new(),
            snippet: name.into(),
        }
    }

    #[test]
    fn symbol_finds_exact_names_only_and_case_sensitive_ones_first() {
        let mut lesson = symbol("User", "lessons/user.md");
        lesson.kind = Kind::Lesson;
        let entries = vec![
            symbol("App\\Models\\User", "app/Models/User.php"),
            symbol("App\\Models\\UserRole", "app/Models/UserRole.php"),
            symbol("com.shop.user", "src/main/kotlin/com/shop/user.kt"),
            symbol("com.shop.Order", "src/main/java/com/shop/Order.java"),
            lesson,
        ];
        assert_eq!(symbol_matches(&entries, "User"), [0]);
        assert_eq!(symbol_matches(&entries, "\\App\\Models\\User"), [0]);
        assert_eq!(symbol_matches(&entries, "user"), [2]);
        assert_eq!(symbol_matches(&entries, "USER"), [0, 2]);
        assert_eq!(symbol_matches(&entries, "com.shop.Order"), [3]);
        assert!(symbol_matches(&entries, "Use").is_empty());
        assert!(symbol_matches(&entries, "Role").is_empty());
    }

    #[test]
    fn hits_are_best_first_and_capped() {
        let entries: Vec<Entry> = (0..30)
            .map(|i| symbol(&format!("N{i:02}"), "a.php"))
            .collect();
        let scored = (0..30).map(|i| (i, i as f64)).collect();
        let hits = rank(&entries, scored, 20);
        assert_eq!(hits.len(), 20);
        assert_eq!(hits[0].name, "N29");
        assert_eq!(hits[19].name, "N10");
    }
}
