//! `fairlead context`: the brief for a change, then what to read before
//! editing it: the bodies of the lessons it offers, the skills to load, the
//! nearest README's first section, and the commits that last touched each
//! file, which is most of the answer to "why is this code like this".

use std::path::Path;
use std::process::ExitCode;

use fairlead_tests::git;
use serde::Serialize;

use crate::brief_cmd::{self, Brief};
use crate::knowledge::{lesson, secrets, skill};

/// The text stops here unless `--all` is given, its last line saying how much is left.
const CAP: usize = 120;
/// READMEs whose first section is shown.
const DOCS: usize = 3;
/// Lines of a README section shown unless `--all` is given.
const DOC_LINES: usize = 12;
/// Commits shown for each file.
const COMMITS: usize = 3;

#[derive(clap::Args)]
pub struct ContextArgs {
    /// The files or directories the change will touch, as for `fairlead brief`.
    #[arg(required = true, num_args = 1..)]
    paths: Vec<String>,
    /// The branch or commit the change starts from; its merge base with HEAD is used.
    #[arg(long)]
    base: Option<String>,
    /// The agent session the brief belongs to, over `CLAUDE_CODE_SESSION_ID`.
    #[arg(long)]
    session: Option<String>,
    /// Print everything instead of stopping at 120 lines.
    #[arg(long)]
    all: bool,
    /// Print the brief and the rest as JSON.
    #[arg(long)]
    json: bool,
    /// Override a config value for this run.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    sets: Vec<String>,
}

#[derive(Serialize)]
pub struct Context {
    pub brief: Brief,
    pub lessons: Vec<LessonBody>,
    pub skills: Vec<SkillRef>,
    pub docs: Vec<DocSection>,
    pub history: Vec<History>,
}

#[derive(Serialize)]
pub struct LessonBody {
    pub id: String,
    /// The title and why the brief offered it.
    pub why: String,
    pub path: String,
    pub body: String,
}

#[derive(Serialize)]
pub struct SkillRef {
    pub name: String,
    pub description: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct DocSection {
    /// The README, from the repository root.
    pub path: String,
    pub heading: String,
    pub lines: Vec<String>,
}

#[derive(Serialize)]
pub struct History {
    pub path: String,
    /// `hash date subject`, newest first.
    pub commits: Vec<String>,
}

pub fn run(args: ContextArgs, cwd: &Path) -> ExitCode {
    match context(&args, cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("context: {e}");
            ExitCode::from(2)
        }
    }
}

fn context(args: &ContextArgs, cwd: &Path) -> Result<(), String> {
    let (brief, planned) = brief_cmd::build(
        cwd,
        &args.paths,
        args.base.as_deref(),
        args.session.clone(),
        &args.sets,
    )?;
    let root = crate::graph_cmd::repo_root(cwd);
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let (lessons, _) = lesson::load(&root, &planned.config.memory);
    let lessons = brief
        .lessons
        .items
        .iter()
        .filter_map(|item| {
            let l = lessons.iter().find(|l| l.front.id == item.name)?;
            Some(LessonBody {
                id: item.name.clone(),
                why: item.why.clone(),
                path: l.path.clone(),
                body: l.body.clone(),
            })
        })
        .collect();
    let (skills, _) = skill::load(&root, &planned.config.skills);
    let skills = brief
        .skills
        .items
        .iter()
        .filter_map(|item| {
            let s = skills.iter().find(|s| s.name == item.name)?;
            Some(SkillRef {
                name: s.name.clone(),
                description: s.description.clone(),
                path: s.path.clone(),
            })
        })
        .collect();
    let docs = docs(&root, &brief.paths);
    let mut ctx = Context {
        brief,
        lessons,
        skills,
        docs,
        history: Vec::new(),
    };
    let existing: Vec<String> = ctx
        .brief
        .paths
        .iter()
        .filter(|p| root.join(p).is_file())
        .cloned()
        .collect();
    if args.json {
        ctx.history = existing.iter().map(|p| history(&root, p)).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&ctx).expect("context serializes")
        );
    } else {
        let cap = (!args.all).then_some(CAP);
        print!("{}", text(&ctx, cap, &existing, &|p| history(&root, p)));
    }
    Ok(())
}

fn history(root: &Path, path: &str) -> History {
    History {
        path: path.to_string(),
        commits: git::recent_commits(root, path, COMMITS)
            .into_iter()
            .filter(|c| secrets::scan(c).is_empty())
            .collect(),
    }
}

/// The nearest README.md above each path, once each, first `DOCS` of them.
fn docs(root: &Path, paths: &[String]) -> Vec<DocSection> {
    let mut seen: Vec<String> = Vec::new();
    for p in paths {
        let mut dir = Path::new(p).parent();
        while let Some(d) = dir {
            let readme = if d.as_os_str().is_empty() {
                "README.md".to_string()
            } else {
                format!("{}/README.md", d.display())
            };
            if root.join(&readme).is_file() {
                if !seen.contains(&readme) {
                    seen.push(readme);
                }
                break;
            }
            dir = d.parent();
        }
        if seen.len() == DOCS {
            break;
        }
    }
    seen.into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(root.join(&path)).ok()?;
            let (heading, lines) = first_section(&text)?;
            // A README isn't checked when it's committed, so a section that looks like it holds a secret is left out.
            let lines = match secrets::scan(&lines.join("\n")).first() {
                Some(found) => vec![format!("left out: it looks like it holds {}", found.kind)],
                None => lines,
            };
            Some(DocSection {
                path,
                heading,
                lines,
            })
        })
        .collect()
}

/// The first heading and the lines under it, or the text before any heading.
fn first_section(text: &str) -> Option<(String, Vec<String>)> {
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty()).peekable();
    let first = *lines.peek()?;
    let heading = if first.starts_with('#') {
        lines.next();
        first.trim_start_matches('#').trim().to_string()
    } else {
        String::new()
    };
    let mut fenced = false;
    let mut body: Vec<String> = Vec::new();
    for line in lines {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && line.starts_with('#') {
            break;
        }
        body.push(line.to_string());
    }
    while body.first().is_some_and(|l| l.trim().is_empty()) {
        body.remove(0);
    }
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    (!heading.is_empty() || !body.is_empty()).then_some((heading, body))
}

/// Lines in blocks; once a block doesn't fit under the cap, it and every
/// later one is counted instead, so what's shown keeps its order.
struct Capped {
    lines: Vec<String>,
    cap: Option<usize>,
    left_out: usize,
}

impl Capped {
    fn full(&self) -> bool {
        self.left_out > 0
    }

    fn block(&mut self, block: Vec<String>) {
        let room = self.cap.map_or(usize::MAX, |c| c - 1);
        if self.full() || self.lines.len() + block.len() > room {
            self.left_out += 1;
        } else {
            self.lines.extend(block);
        }
    }

    fn finish(mut self) -> String {
        if self.full() {
            self.lines
                .push(format!("… {} more (fairlead context --all)", self.left_out));
        }
        self.lines.into_iter().map(|l| l + "\n").collect()
    }
}

/// The context as an agent reads it: the brief, then a block per lesson,
/// skill, README section and file history, under `cap` lines. A file's
/// history is only asked of git while it can still be shown.
pub fn text(
    ctx: &Context,
    cap: Option<usize>,
    existing: &[String],
    history: &dyn Fn(&str) -> History,
) -> String {
    let all = cap.is_none();
    let brief: Vec<String> = brief_cmd::text(&ctx.brief, all)
        .lines()
        .map(str::to_string)
        .collect();
    let room = cap.map_or(brief.len(), |c| brief.len().min(c - 1));
    let mut out = Capped {
        lines: brief[..room].to_vec(),
        cap,
        left_out: usize::from(room < brief.len()),
    };
    for l in &ctx.lessons {
        let mut block = vec![format!("lesson   {}  {}", l.id, l.why)];
        block.extend(l.body.lines().map(|b| format!("  {b}")));
        out.block(block);
    }
    for s in &ctx.skills {
        let about = if s.description.is_empty() {
            String::new()
        } else {
            format!("  {}", s.description)
        };
        out.block(vec![
            format!("skill    {}{about}", s.name),
            format!("  load it before editing: {}", s.path),
        ]);
    }
    for d in &ctx.docs {
        let mut block = vec![format!("docs     {}  {}", d.path, d.heading)];
        let shown = if all {
            d.lines.len()
        } else {
            d.lines.len().min(DOC_LINES)
        };
        block.extend(d.lines[..shown].iter().map(|l| format!("  {l}")));
        if shown < d.lines.len() {
            block.push(format!("  … the rest is in {}", d.path));
        }
        out.block(block);
    }
    for p in existing {
        if out.full() {
            out.left_out += 1;
            continue;
        }
        let h = history(p);
        if h.commits.is_empty() {
            continue;
        }
        let mut block = vec![format!("history  {}", h.path)];
        block.extend(h.commits.iter().map(|c| format!("  {c}")));
        out.block(block);
    }
    out.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_section_is_the_first_heading_and_what_it_holds() {
        let text = "\n# Forms\n\nHow forms validate.\n```\n# not a heading\n```\n\n## Next\nmore\n";
        let (heading, lines) = first_section(text).unwrap();
        assert_eq!(heading, "Forms");
        assert_eq!(
            lines,
            ["How forms validate.", "```", "# not a heading", "```"]
        );
        let (heading, lines) = first_section("Just text.\n# Later\n").unwrap();
        assert_eq!((heading.as_str(), lines.len()), ("", 1));
        assert!(first_section("\n\n").is_none());
    }

    #[test]
    fn a_block_that_does_not_fit_is_counted_with_every_later_one() {
        let mut out = Capped {
            lines: vec!["brief".into()],
            cap: Some(4),
            left_out: 0,
        };
        out.block(vec!["a".into(), "b".into()]);
        out.block(vec!["c".into(), "d".into()]);
        out.block(vec!["e".into()]);
        assert_eq!(
            out.finish(),
            "brief\na\nb\n… 2 more (fairlead context --all)\n"
        );
    }
}
