//! `fairlead agents sync` keeps a marked block in the files agents read
//! first: the change loop's commands, the always-on lessons and the skill
//! index. Nothing outside the markers is written; `doctor` names a file
//! whose block is missing or stale.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::config::{AgentsWrite, Config, LoadOptions};

use crate::knowledge::{block, lesson, short, skill};

#[derive(clap::Subcommand)]
pub enum AgentsAction {
    /// Write the block between `<!-- fairlead:begin -->` and `<!-- fairlead:end -->` in each of `agents.files`, adding it at the end of a file without one.
    Sync {
        /// Write nothing; exit 1 naming each file whose block is missing or stale.
        #[arg(long, conflicts_with = "clean")]
        check: bool,
        /// Remove the block and its markers, leaving the rest of each file as it was.
        #[arg(long)]
        clean: bool,
    },
}

/// What a file holds, against the block it should.
enum State {
    Current,
    Missing,
    NoBlock,
    Stale,
    Broken(String),
}

impl State {
    fn of(text: Result<Option<String>, String>, lines: &[String]) -> State {
        let text = match text {
            Ok(Some(t)) => t,
            Ok(None) => return State::Missing,
            Err(e) => return State::Broken(e),
        };
        match block::locate(&text) {
            Err(e) => State::Broken(e),
            Ok(None) => State::NoBlock,
            Ok(Some(_)) if block::splice(Some(&text), lines).as_ref() == Ok(&text) => {
                State::Current
            }
            Ok(Some(_)) => State::Stale,
        }
    }

    /// What's wrong, as "FILE …; fix", or none when the block is current.
    fn problem(&self, file: &str, write: AgentsWrite) -> Option<String> {
        let fix = match write {
            AgentsWrite::Block => "`fairlead agents sync` writes it",
            AgentsWrite::Never => "copy in the block `fairlead agents sync` prints",
        };
        match self {
            State::Current => None,
            State::Missing => Some(format!("{file} doesn't exist; {fix}")),
            State::NoBlock => Some(format!("{file} has no fairlead block; {fix}")),
            State::Stale => Some(format!("{file}'s fairlead block is stale; {fix}")),
            State::Broken(why) => Some(format!("{file}: {why}")),
        }
    }
}

fn root_and_config(cwd: &Path) -> Result<(PathBuf, Config), String> {
    let loaded = fairlead_core::config::load(cwd, &LoadOptions::from_process(Vec::new()))
        .map_err(|e| e.to_string())?;
    if let Some(p) = loaded
        .problems
        .iter()
        .find(|p| p.key.starts_with("agents."))
    {
        return Err(format!("{}: {}", p.key, p.message));
    }
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    Ok((root, loaded.config))
}

/// The block's lines for this repository, and the lesson and skill files
/// it had to leave out.
pub fn lines(root: &Path, config: &Config) -> (Vec<String>, Vec<String>) {
    let (lessons, bad_lessons) = lesson::load(root, &config.memory);
    let mut always: Vec<&lesson::Lesson> = lessons.iter().filter(|l| l.front.always).collect();
    always.sort_by(|a, b| a.front.id.cmp(&b.front.id));
    let (mut skills, bad_skills) = skill::load(root, &config.skills);
    skills.sort_by(|a, b| (&a.name, &a.path).cmp(&(&b.name, &b.path)));
    let lists = block::Lists {
        lessons: always
            .iter()
            .map(|l| block::item(&format!("{} ({})", l.front.title, l.front.id)))
            .collect(),
        skills: skills
            .iter()
            .map(|s| match s.description.as_str() {
                "" => block::item(&format!("{} ({})", s.name, s.path)),
                d => block::item(&format!("{}: {} ({})", s.name, short(d, 100), s.path)),
            })
            .collect(),
    };
    let mut bad: Vec<String> = bad_lessons
        .iter()
        .map(|b| format!("[bad-lesson] {}: {}", b.path, b.reason))
        .collect();
    bad.extend(
        bad_skills
            .iter()
            .map(|b| format!("[bad-skill] {}: {}", b.path, b.reason)),
    );
    (block::render(&config.memory.dir, &lists), bad)
}

/// Each file once, in the order the config lists them.
fn files(config: &Config) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in config.agents.files.items() {
        let f = f.trim_start_matches("./").to_string();
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "isn't UTF-8 text, so it's left untouched".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// The doctor's lines: each file whose block is missing, stale or broken,
/// in either write mode, or one line saying they're all current.
pub fn doctor_lines(root: &Path, config: &Config) -> String {
    let (lines, _) = lines(root, config);
    let files = files(config);
    let problems: Vec<String> = files
        .iter()
        .filter_map(|f| State::of(read(&root.join(f)), &lines).problem(f, config.agents.write))
        .map(|p| format!("agents: {p}\n"))
        .collect();
    match (problems.is_empty(), files.is_empty()) {
        (_, true) => "agents: no files; `agents.files` is empty\n".to_string(),
        (true, false) => format!(
            "agents: the fairlead block is current in {}\n",
            files.join(", ")
        ),
        (false, false) => problems.concat(),
    }
}

pub fn run(action: AgentsAction, cwd: &Path) -> ExitCode {
    let AgentsAction::Sync { check, clean } = action;
    let (root, config) = match root_and_config(cwd) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("agents: {e}");
            return ExitCode::from(2);
        }
    };
    let (lines, bad) = lines(&root, &config);
    for b in &bad {
        eprintln!("agents: warning {b}");
    }
    let files = files(&config);
    let never = config.agents.write == AgentsWrite::Never;
    match (check, clean, never) {
        (true, _, _) => check_files(&root, &files, &lines, config.agents.write),
        (_, true, true) => {
            eprintln!("agents: agents.write = \"never\", so nothing was removed");
            ExitCode::SUCCESS
        }
        (_, true, false) => clean_files(&root, &files),
        (_, _, true) => {
            println!("{}", lines.join("\n"));
            eprintln!(
                "agents: agents.write = \"never\", so nothing was written; a person copies the block above into {}",
                files.join(", ")
            );
            ExitCode::SUCCESS
        }
        _ => sync_files(&root, &files, &lines),
    }
}

fn check_files(root: &Path, files: &[String], lines: &[String], write: AgentsWrite) -> ExitCode {
    let problems: Vec<String> = files
        .iter()
        .filter_map(|f| State::of(read(&root.join(f)), lines).problem(f, write))
        .collect();
    for p in &problems {
        println!("agents: {p}");
    }
    if problems.is_empty() {
        println!(
            "agents: the fairlead block is current in {}",
            files.join(", ")
        );
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn sync_files(root: &Path, files: &[String], lines: &[String]) -> ExitCode {
    let mut failed = false;
    for f in files {
        let path = root.join(f);
        let outcome = read(&path).and_then(|text| {
            let new = block::splice(text.as_deref(), lines)?;
            if text.as_deref() == Some(new.as_str()) {
                return Ok(format!("{f} is current"));
            }
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, &new).map_err(|e| e.to_string())?;
            Ok(match text {
                None => format!("created {f} with the block"),
                Some(t) if block::locate(&t)?.is_some() => format!("updated the block in {f}"),
                Some(_) => format!("added the block at the end of {f}"),
            })
        });
        match outcome {
            Ok(done) => println!("agents: {done}"),
            Err(e) => {
                failed = true;
                eprintln!("agents: {f}: {e}; left untouched");
            }
        }
    }
    if failed {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn clean_files(root: &Path, files: &[String]) -> ExitCode {
    let mut failed = false;
    for f in files {
        let path = root.join(f);
        let outcome = read(&path).and_then(|text| {
            let Some(text) = text else {
                return Ok(format!("{f} doesn't exist"));
            };
            let Some(rest) = block::clean(&text)? else {
                return Ok(format!("{f} has no block"));
            };
            // Sync creates a file as the block and a newline; one it found
            // empty keeps no final newline, so it stays.
            if rest.is_empty() && text.ends_with('\n') {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                return Ok(format!("removed {f}, which held only the block"));
            }
            std::fs::write(&path, rest).map_err(|e| e.to_string())?;
            Ok(format!("removed the block from {f}"))
        });
        match outcome {
            Ok(done) => println!("agents: {done}"),
            Err(e) => {
                failed = true;
                eprintln!("agents: {f}: {e}; left untouched");
            }
        }
    }
    if failed {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
