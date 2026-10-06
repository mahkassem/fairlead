//! `fairlead skills eval`: how much of what history says a change needed
//! the router would have offered. A commit that changes code and edits a
//! routed SKILL.md needed that skill; each way of reaching past the changed
//! paths is scored on those commits, against one graph of the current tree.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, ExitCode};

use fairlead_core::config::{self, LoadOptions};
use fairlead_lang::graph::Graph;
use serde::Serialize;

use crate::knowledge::route::{self, Hops, Reach};
use crate::knowledge::skill::{self, Skill};

#[derive(clap::Subcommand)]
pub enum SkillsAction {
    /// Score skill routing on git history: a commit that changes code and edits a routed SKILL.md needed that skill.
    Eval {
        /// Every first-parent commit since this date, as `git log --since` reads it.
        #[arg(long, value_name = "DATE", conflicts_with = "limit")]
        since: Option<String>,
        /// How many first-parent commits from HEAD to read (500).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
        /// Print the scores as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Each way of reaching past the changed paths, as (name, hops).
pub const METHODS: [(&str, Hops); 4] = [
    (
        "paths",
        Hops {
            imports: 0,
            importers: 0,
        },
    ),
    (
        "imports",
        Hops {
            imports: 1,
            importers: 0,
        },
    ),
    (
        "importers",
        Hops {
            imports: 0,
            importers: 1,
        },
    ),
    (
        "both",
        Hops {
            imports: 1,
            importers: 1,
        },
    ),
];
/// The method `[skills]` defaults to, whose ranking the caps cut.
const DEFAULT: usize = 1;
pub const CAPS: [usize; 4] = [5, 6, 8, 10];
const LIMIT: usize = 500;

/// One commit: its id and each file it changed, with git's status letter.
#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub id: String,
    pub files: Vec<(char, String)>,
}

/// A commit history can score: the skills it edited, and the other files it changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub id: String,
    pub needed: BTreeSet<usize>,
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MethodScore {
    pub method: String,
    pub imports: usize,
    pub importers: usize,
    pub commits: usize,
    pub needed: usize,
    pub hit: usize,
    pub offered: usize,
    pub recall: f64,
    pub covered: usize,
    pub offered_per_change: f64,
    pub precision: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CapScore {
    pub cap: usize,
    pub hit: usize,
    pub recall: f64,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub head: String,
    pub walked: usize,
    pub scored: usize,
    pub skills: usize,
    pub graph: String,
    pub methods: Vec<MethodScore>,
    pub default: String,
    pub caps: Vec<CapScore>,
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `git log --name-status -z` with each commit led by `\x1e`: the id, then
/// status and path pairs. Renames come as a delete and an add.
pub fn parse_log(raw: &str) -> Vec<Commit> {
    raw.split('\x1e')
        .filter(|c| !c.trim().is_empty())
        .map(|chunk| {
            let mut fields = chunk.split('\0').map(|f| f.trim_start_matches('\n'));
            let id = fields.next().unwrap_or_default().trim().to_string();
            let mut files = Vec::new();
            while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
                if let Some(s) = status.chars().next() {
                    files.push((s, path.to_string()));
                }
            }
            Commit { id, files }
        })
        .collect()
}

/// First-parent commits from HEAD, newest first; a merge counts as its diff
/// against the first parent.
pub fn history(root: &Path, since: Option<&str>, limit: usize) -> Result<Vec<Commit>, String> {
    let mut args = vec![
        "log".to_string(),
        "--first-parent".into(),
        "--diff-merges=first-parent".into(),
        "--no-renames".into(),
        "--name-status".into(),
        "-z".into(),
        "--format=%x1e%H".into(),
    ];
    match since {
        Some(date) => args.push(format!("--since={date}")),
        None => args.push(format!("--max-count={limit}")),
    }
    args.extend(["HEAD".into(), "--".into()]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(parse_log(&git(root, &args)?))
}

/// The commits that both modified a routed SKILL.md and changed a file
/// outside every routed skill. A SKILL.md added in the commit isn't needed
/// by it: the router had nothing to offer yet.
pub fn cases(commits: &[Commit], skills: &[Skill]) -> Vec<Case> {
    commits
        .iter()
        .filter_map(|c| {
            let needed: BTreeSet<usize> = c
                .files
                .iter()
                .filter(|(s, _)| *s == 'M')
                .filter_map(|(_, f)| {
                    skills
                        .iter()
                        .position(|k| k.path.trim_start_matches("./") == f)
                })
                .collect();
            let changed: Vec<String> = c
                .files
                .iter()
                .filter(|(_, f)| skill::by_path(skills, f).is_none())
                .map(|(_, f)| f.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            (!needed.is_empty() && !changed.is_empty()).then(|| Case {
                id: c.id.clone(),
                needed,
                changed,
            })
        })
        .collect()
}

/// The skills a change is offered, in the brief's order.
pub fn ranked(
    case: &Case,
    skills: &[Skill],
    graph: &Graph,
    module_of: &dyn Fn(&str) -> Option<String>,
    hops: Hops,
) -> Vec<usize> {
    let reach = Reach::new(&case.changed, graph, hops, module_of, false);
    route::select(skills, |s| &s.scope, |s| s.name.clone(), &reach)
        .into_iter()
        .map(|(i, _)| i)
        .collect()
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        return 0.0;
    }
    (a as f64 / b as f64 * 10_000.0).round() / 10_000.0
}

/// One method's numbers over (needed, offered in order) per commit.
pub fn tally(name: &str, hops: Hops, results: &[(&BTreeSet<usize>, Vec<usize>)]) -> MethodScore {
    let (mut needed, mut hit, mut offered, mut covered) = (0, 0, 0, 0);
    for (need, got) in results {
        let found = need.iter().filter(|n| got.contains(n)).count();
        needed += need.len();
        hit += found;
        offered += got.len();
        covered += usize::from(found == need.len());
    }
    MethodScore {
        method: name.to_string(),
        imports: hops.imports,
        importers: hops.importers,
        commits: results.len(),
        needed,
        hit,
        offered,
        recall: ratio(hit, needed),
        covered,
        offered_per_change: ratio(offered, results.len()),
        precision: ratio(hit, offered),
    }
}

/// Recall when only the first `cap` offered count, as a brief shows them.
pub fn capped(results: &[(&BTreeSet<usize>, Vec<usize>)], cap: usize) -> CapScore {
    let needed: usize = results.iter().map(|(n, _)| n.len()).sum();
    let hit = results
        .iter()
        .map(|(need, got)| got.iter().take(cap).filter(|g| need.contains(g)).count())
        .sum();
    CapScore {
        cap,
        hit,
        recall: ratio(hit, needed),
    }
}

/// Every method's score on `cases`, and the caps on the default's ranking.
pub fn score(
    cases: &[Case],
    skills: &[Skill],
    graph: &Graph,
    module_of: &dyn Fn(&str) -> Option<String>,
) -> (Vec<MethodScore>, Vec<CapScore>) {
    let mut methods = Vec::new();
    let mut caps = Vec::new();
    for (i, (name, hops)) in METHODS.iter().enumerate() {
        let results: Vec<(&BTreeSet<usize>, Vec<usize>)> = cases
            .iter()
            .map(|c| (&c.needed, ranked(c, skills, graph, module_of, *hops)))
            .collect();
        methods.push(tally(name, *hops, &results));
        if i == DEFAULT {
            caps = CAPS.iter().map(|&cap| capped(&results, cap)).collect();
        }
    }
    (methods, caps)
}

pub fn run(action: SkillsAction, cwd: &Path) -> ExitCode {
    let SkillsAction::Eval { since, limit, json } = action;
    match eval(cwd, since.as_deref(), limit.unwrap_or(LIMIT)) {
        Ok(report) if json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("report prints")
            );
            ExitCode::SUCCESS
        }
        Ok(report) => {
            print!("{}", render(&report));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("skills eval: {e}");
            ExitCode::from(2)
        }
    }
}

fn eval(cwd: &Path, since: Option<&str>, limit: usize) -> Result<Report, String> {
    let loaded =
        config::load(cwd, &LoadOptions::from_process(Vec::new())).map_err(|e| e.to_string())?;
    if !loaded.problems.is_empty() {
        return Err("the config has problems; run `fairlead config check`".into());
    }
    let root = if loaded.files.is_empty() {
        crate::graph_cmd::repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    let (skills, bad) = skill::load(&root, &loaded.config.skills);
    for b in &bad {
        eprintln!("warning [bad-skill]: {}: {}", b.path, b.reason);
    }
    if skills.is_empty() {
        return Err(
            "no routed skill to score; add `[[skills.routes]]` or run `fairlead import rules`"
                .into(),
        );
    }
    let commits = history(&root, since, limit)?;
    let cases = cases(&commits, &skills);
    let scan = fairlead_lang::build(&root, &loaded.config)
        .map_err(|e| format!("could not read {}: {e}", root.display()))?;
    let modules = fairlead_tests::modules::Modules::discover(
        &scan.tree,
        &scan.packages,
        &loaded.config.modules,
    )
    .unwrap_or_default();
    let module_of = |p: &str| modules.name_of(p).map(str::to_string);
    let (methods, caps) = score(&cases, &skills, &scan.graph, &module_of);
    Ok(Report {
        head: fairlead_tests::git::head(&root).unwrap_or_default(),
        walked: commits.len(),
        scored: cases.len(),
        skills: skills.len(),
        graph: "the working tree's import graph, built once: each commit is routed along today's imports, not its own".into(),
        methods,
        default: METHODS[DEFAULT].0.into(),
        caps,
    })
}

fn pct(r: f64) -> String {
    format!("{:.1}%", r * 100.0)
}

pub fn render(r: &Report) -> String {
    let short: String = r.head.chars().take(12).collect();
    let mut out = format!(
        "skills eval: {} of {} first-parent commits from {short} scored, {} routed skill(s)\n",
        r.scored, r.walked, r.skills
    );
    out.push_str(&format!("graph: {}\n", r.graph));
    if r.scored == 0 {
        out.push_str("no commit both changed code and modified a routed SKILL.md, so there's nothing to score\n");
        return out;
    }
    out.push_str(&format!(
        "\n{:<11}{:<6}{:<20}{:<10}{:<10}{}\n",
        "method", "hops", "recall", "covered", "offered", "precision"
    ));
    for m in &r.methods {
        let star = if m.method == r.default { "*" } else { "" };
        let name = format!("{}{star}", m.method);
        let hops = format!("{}/{}", m.imports, m.importers);
        let recall = format!("{} ({}/{})", pct(m.recall), m.hit, m.needed);
        let covered = format!("{}/{}", m.covered, m.commits);
        let precision = format!("{} ({}/{})", pct(m.precision), m.hit, m.offered);
        out.push_str(&format!(
            "{name:<11}{hops:<6}{recall:<20}{covered:<10}{:<10.1}{precision}\n",
            m.offered_per_change,
        ));
    }
    out.push_str(&format!(
        "* the default; hops are imports/importers; covered is commits with every needed skill offered; offered is per change\n\nrecall at a cap, {} ranked as the brief ranks:",
        r.default
    ));
    for c in &r.caps {
        out.push_str(&format!("  {}: {}", c.cap, pct(c.recall)));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_parses_into_commits_with_statuses() {
        let raw =
            "\x1eaaa\0\nM\0src/a.ts\0A\0.claude/skills/x/SKILL.md\0\x1ebbb\0\x1eccc\0\nD\0old.ts\0";
        let commits = parse_log(raw);
        assert_eq!(commits.len(), 3);
        assert_eq!(commits[0].id, "aaa");
        assert_eq!(
            commits[0].files,
            [
                ('M', "src/a.ts".into()),
                ('A', ".claude/skills/x/SKILL.md".into())
            ]
        );
        assert!(commits[1].files.is_empty());
        assert_eq!(commits[2].files, [('D', "old.ts".into())]);
    }

    fn set(items: &[usize]) -> BTreeSet<usize> {
        items.iter().copied().collect()
    }

    #[test]
    fn recall_precision_and_cover_count_what_was_needed_and_offered() {
        let (a, b) = (set(&[0]), set(&[1, 2]));
        let results = vec![(&a, vec![3, 0]), (&b, vec![1])];
        let m = tally(
            "x",
            Hops {
                imports: 1,
                importers: 0,
            },
            &results,
        );
        assert_eq!((m.needed, m.hit, m.offered, m.covered), (3, 2, 3, 1));
        assert_eq!(m.recall, 0.6667);
        assert_eq!(m.precision, 0.6667);
        assert_eq!(m.offered_per_change, 1.5);
    }

    #[test]
    fn a_cap_counts_only_the_first_offered() {
        let need = set(&[9]);
        let ranked: Vec<usize> = (0..10).collect();
        let results = vec![(&need, ranked)];
        assert_eq!(capped(&results, 8).hit, 0, "9 is tenth");
        assert_eq!(capped(&results, 10).hit, 1);
    }
}
