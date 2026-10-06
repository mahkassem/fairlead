//! `fairlead learn` writes one lesson; `fairlead lessons` lists them, the
//! ones due for review, and the files that can't be offered.

use std::io::{IsTerminal, Read};
use std::path::Path;
use std::process::ExitCode;

use fairlead_core::config::{Learn, LoadOptions, Memory};

use crate::knowledge::lesson::{self, Front, Lesson, Source};

#[derive(clap::Args)]
pub struct LearnArgs {
    /// One line: what to do or not do.
    #[arg(long)]
    title: String,
    /// The file name and id; the title's words when left out.
    #[arg(long)]
    id: Option<String>,
    /// A glob of the files it applies to; repeat for more.
    #[arg(long = "path", value_name = "GLOB")]
    paths: Vec<String>,
    /// A module it applies to; repeat for more.
    #[arg(long = "module", value_name = "NAME")]
    modules: Vec<String>,
    /// Offer it for every change.
    #[arg(long)]
    always: bool,
    /// Offer it only when `fairlead find` matches it, for a lesson that names no code.
    #[arg(long)]
    search: bool,
    /// A link to where it was learned: a pull request, a CI run, a review; repeat for more.
    #[arg(long = "evidence", value_name = "LINK", required = true)]
    evidence: Vec<String>,
    /// `person`: someone confirmed it (with --confirmed-by); `mistake`: the evidence is a failure anyone can see.
    #[arg(long, value_parser = ["person", "mistake"])]
    source: String,
    /// Who confirmed it, for `--source person`.
    #[arg(long)]
    confirmed_by: Option<String>,
    /// The rule or test that enforces it.
    #[arg(long)]
    check: Option<String>,
    /// The body; read from stdin when left out.
    #[arg(long)]
    body: Option<String>,
}

#[derive(clap::Subcommand)]
pub enum LessonsAction {
    /// Every lesson with its scope, or only those past their review date.
    List {
        #[arg(long)]
        due: bool,
        #[arg(long)]
        json: bool,
    },
    /// The lessons past their review date; still offered, marked due.
    Review,
    /// Fail when a lesson file can't be offered: a missing scope, evidence or source, a long body, or a secret.
    Check,
}

pub(crate) fn memory_at(cwd: &Path) -> Result<(std::path::PathBuf, Memory), String> {
    let loaded = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
        .map_err(|e| e.to_string())?;
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    Ok((root, loaded.config.memory))
}

pub fn learn(args: LearnArgs, cwd: &Path) -> ExitCode {
    match learn_inner(args, cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("learn: {e}");
            ExitCode::from(2)
        }
    }
}

fn learn_inner(args: LearnArgs, cwd: &Path) -> Result<(), String> {
    let (root, memory) = memory_at(cwd)?;
    let body = match args.body {
        Some(b) => b,
        None if !std::io::stdin().is_terminal() => {
            let mut b = String::new();
            std::io::stdin()
                .read_to_string(&mut b)
                .map_err(|e| e.to_string())?;
            b
        }
        None => return Err("give the body with --body or on stdin".into()),
    };
    let added = lesson::today();
    let front = Front {
        id: args.id.unwrap_or_else(|| lesson::slug(&args.title)),
        title: args.title,
        paths: args.paths,
        modules: args.modules,
        always: args.always,
        search: args.search,
        review_by: lesson::add_days(&added, i64::from(memory.review_days)),
        added,
        evidence: args.evidence,
        check: args.check,
        source: if args.source == "person" {
            Source::Person
        } else {
            Source::Mistake
        },
        confirmed_by: args.confirmed_by,
    };
    let text = lesson::render(&front, &body);
    let rel = format!("{}/{}.md", memory.dir.trim_end_matches('/'), front.id);
    lesson::parse(&rel, &text, &memory).map_err(|p| p.join("; "))?;
    let path = root.join(&rel);
    if path.exists() {
        return Err(format!("{rel} already exists; give another --id"));
    }
    match memory.learn {
        Learn::Ask => {
            print!("{text}");
            eprintln!("learn: memory.learn = \"ask\", so nothing was written; save it as {rel} to keep it");
        }
        Learn::Write => {
            std::fs::create_dir_all(path.parent().expect("a lesson has a directory"))
                .map_err(|e| e.to_string())?;
            std::fs::write(&path, text).map_err(|e| format!("{rel}: {e}"))?;
            println!("learn: wrote {rel}; it's reviewed with the pull request it rides in");
        }
    }
    Ok(())
}

pub fn run(action: LessonsAction, cwd: &Path) -> ExitCode {
    let (root, memory) = match memory_at(cwd) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("lessons: {e}");
            return ExitCode::from(2);
        }
    };
    let (lessons, bad) = lesson::load(&root, &memory);
    let today = lesson::today();
    match action {
        LessonsAction::Check => {
            for b in &bad {
                println!("[bad-lesson] {}: {}", b.path, b.reason);
            }
            println!(
                "lessons: {} fine, {} that can't be offered",
                lessons.len(),
                bad.len()
            );
            if bad.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        LessonsAction::Review => {
            list(&lessons, &today, true, false);
            ExitCode::SUCCESS
        }
        LessonsAction::List { due, json } => {
            list(&lessons, &today, due, json);
            for b in &bad {
                eprintln!("[bad-lesson] {}: {}", b.path, b.reason);
            }
            ExitCode::SUCCESS
        }
    }
}

fn list(lessons: &[Lesson], today: &str, due_only: bool, json: bool) {
    let shown: Vec<&Lesson> = lessons
        .iter()
        .filter(|l| !due_only || l.due(today))
        .collect();
    if json {
        let rows: Vec<serde_json::Value> = shown
            .iter()
            .map(|l| {
                serde_json::json!({
                    "path": l.path,
                    "due": l.due(today),
                    "lesson": l.front,
                    "body": l.body,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).expect("rows serialize")
        );
        return;
    }
    for l in &shown {
        println!(
            "{}{}  {}  [{}]",
            l.front.id,
            if l.due(today) { " (due)" } else { "" },
            l.front.title,
            scope_text(&l.front)
        );
    }
    if shown.is_empty() {
        println!(
            "lessons: {}",
            if due_only {
                "none due for review"
            } else {
                "none yet; `fairlead learn` writes one"
            }
        );
    }
}

fn scope_text(f: &Front) -> String {
    let mut parts: Vec<String> = f.paths.clone();
    parts.extend(f.modules.iter().map(|m| format!("module {m}")));
    if f.always {
        parts.push("always".into());
    }
    if f.search {
        parts.push("search only".into());
    }
    parts.join(", ")
}

/// One doctor line about the lessons, or none when there are none.
pub fn doctor_line(root: &Path, memory: &Memory) -> Option<String> {
    let (lessons, bad) = lesson::load(root, memory);
    if lessons.is_empty() && bad.is_empty() {
        return None;
    }
    let today = lesson::today();
    let due = lessons.iter().filter(|l| l.due(&today)).count();
    let mut line = format!(
        "lessons: {} in {} ({due} due for review)\n",
        lessons.len(),
        memory.dir
    );
    for b in &bad {
        line.push_str(&format!("  [bad-lesson] {}: {}\n", b.path, b.reason));
    }
    Some(line)
}
