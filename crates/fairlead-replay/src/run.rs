//! Replaying recorded failures: each failed run's commit is checked out and
//! planned from the merge base of its base and head, and every failing test
//! file and check it names is classed against that plan.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use fairlead_core::config::Config;
use fairlead_core::plan::Plan;
use fairlead_lang::build;
use fairlead_tests::{git as plan_git, plan, Input};
use regex::Regex;

use crate::attribute::{attribute, Attribution, Repo};
use crate::dataset::{Job, Row};
use crate::extract::{extract, Extractor};
use crate::git::{has_commit, merge_base, patch_id, tree_of, Worktree};
use crate::window::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    /// In the plan.
    Hit,
    /// Not in the plan, and nothing explains it away.
    Miss,
    /// Another attempt of the same run passed the same job.
    Flaky,
    /// A later run of the same change (a re-run or a rebase) passed the job.
    Unconfirmed,
    /// The job failed, but no test file or check could be named.
    Unattributed,
    /// The commit, or the history to its merge base, can't be fetched.
    Unavailable,
    /// The planner refused the commit, such as a test with no runner.
    Error,
    /// No `[[replay.failures]]` or `[[replay.checks]]` entry names the job.
    Unwatched,
    /// `replay.ignore` names the job.
    Ignored,
}

/// How a hit's target came to be in the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HitBy {
    Selected,
    RunAll,
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Test(String),
    Check(String),
    Job,
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub run_id: u64,
    pub attempt: u32,
    pub event: String,
    pub pr: Option<u64>,
    pub head_sha: String,
    pub job: String,
    pub target: Target,
    pub outcome: Outcome,
    pub hit_by: Option<HitBy>,
    pub detail: String,
    /// The changed paths of the plan it was judged against.
    pub changed: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Planned {
    pub run_id: u64,
    pub selected: usize,
    pub total: usize,
    pub all: bool,
    pub seconds: f64,
}

#[derive(Debug, Default)]
pub struct Replayed {
    pub failures: Vec<Failure>,
    pub plans: Vec<Planned>,
    pub runs: usize,
}

/// A `[[replay.failures]]` or `[[replay.checks]]` entry, compiled.
pub struct Sources {
    failures: Vec<(Regex, Extractor)>,
    checks: Vec<(Regex, Regex, String)>,
    ignore: Vec<Regex>,
}

impl Sources {
    pub fn new(config: &Config) -> Result<Sources, String> {
        let any = || Regex::new("").expect("empty regex");
        let compile = |p: &Option<String>| {
            p.as_deref()
                .map(Regex::new)
                .transpose()
                .map_err(|e| e.to_string())
        };
        let failures = config
            .replay
            .failures
            .items()
            .iter()
            .map(|f| {
                Ok((
                    compile(&f.job)?.unwrap_or_else(any),
                    Extractor::named(&f.extractor, f.pattern.as_deref())?,
                ))
            })
            .collect::<Result<_, String>>()?;
        let checks = config
            .replay
            .checks
            .items()
            .iter()
            .map(|c| {
                let job = compile(&c.job)?.unwrap_or_else(any);
                let step = Regex::new(&c.step).map_err(|e| e.to_string())?;
                Ok((job, step, c.check.clone()))
            })
            .collect::<Result<_, String>>()?;
        let ignore = config
            .replay
            .ignore
            .items()
            .iter()
            .map(|p| Regex::new(p).map_err(|e| e.to_string()))
            .collect::<Result<_, String>>()?;
        Ok(Sources {
            failures,
            checks,
            ignore,
        })
    }

    fn ignores(&self, job: &Job) -> bool {
        self.ignore.iter().any(|re| re.is_match(&job.name))
    }

    fn watches(&self, job: &Job) -> bool {
        self.failures.iter().any(|(re, _)| re.is_match(&job.name))
            || self.checks.iter().any(|(re, _, _)| re.is_match(&job.name))
    }
}

/// What a failed job names: test files, checks, or nothing it can place.
fn targets(
    job: &Job,
    sources: &Sources,
    repo: &Repo,
    tests: &BTreeSet<String>,
    read: &dyn Fn(&str) -> Option<String>,
) -> Vec<Target> {
    let mut found: Vec<Target> = Vec::new();
    let mut push = |t: Target| {
        if !found.contains(&t) {
            found.push(t);
        }
    };
    for (job_re, extractor) in &sources.failures {
        if !job_re.is_match(&job.name) {
            continue;
        }
        for note in &job.annotations {
            if tests.contains(&note.path) {
                push(Target::Test(note.path.clone()));
            }
        }
        for printed in extract(extractor, &job.log.join("\n")) {
            if let Attribution::File(file) = attribute(&printed, repo, read) {
                if tests.contains(&file) {
                    push(Target::Test(file));
                }
            }
        }
    }
    for (job_re, step_re, check) in &sources.checks {
        if job_re.is_match(&job.name) && job.failed_steps.iter().any(|s| step_re.is_match(s)) {
            push(Target::Check(check.clone()));
        }
    }
    if found.is_empty() {
        found.push(Target::Job);
    }
    found
}

fn in_plan(plan: &Plan, target: &Target) -> Option<HitBy> {
    match target {
        // A plan that selects everything also lists every test.
        Target::Test(_) if plan.all => Some(HitBy::RunAll),
        Target::Test(path) if plan.tests.iter().any(|t| &t.path == path) => Some(HitBy::Selected),
        Target::Check(id) if plan.checks.iter().any(|c| &c.id == id) => Some(HitBy::Check),
        _ => None,
    }
}

/// A plan at a commit, with the test files the tree had.
struct PlanAt {
    plan: Plan,
    tests: BTreeSet<String>,
    seconds: f64,
}

/// The files and workspace packages at the checked-out commit.
struct Snapshot {
    files: Vec<String>,
    packages: Vec<(String, String)>,
}

pub struct Replayer<'a> {
    pub clone: &'a Path,
    pub worktree: Worktree,
    pub config: &'a Config,
    pub sources: Sources,
}

impl Replayer<'_> {
    /// The plan for `base..head` with `head` checked out.
    fn plan_between(&self, base: &str, head: &str) -> Result<PlanAt, String> {
        self.worktree.checkout(head)?;
        let started = Instant::now();
        let mut scan = build(&self.worktree.path, self.config).map_err(|e| e.to_string())?;
        let changes = plan_git::changes(&self.worktree.path, base)?;
        let modules = fairlead_tests::modules::Modules::discover(
            &scan.tree,
            &scan.packages,
            &self.config.modules,
        )?;
        let tests: BTreeSet<String> =
            fairlead_tests::testfiles::discover(&scan.tree, self.config, &modules)?
                .tests
                .into_iter()
                .map(|t| t.path)
                .collect();
        let input = Input {
            changes,
            base: Some(base.to_string()),
            head: head.to_string(),
            config_digest: fairlead_tests::digest::config_digest(self.config),
            tree_hash: tree_of(self.clone, head).unwrap_or_default(),
        };
        let plan = plan(&mut scan, self.config, input)?;
        Ok(PlanAt {
            plan,
            tests,
            seconds: started.elapsed().as_secs_f64(),
        })
    }

    fn snapshot(&self) -> Snapshot {
        let tree = fairlead_lang::tree::Tree::scan(&self.worktree.path);
        let packages = fairlead_lang::workspace::discover(&tree)
            .into_iter()
            .map(|p| (p.name, p.dir))
            .collect();
        Snapshot {
            files: tree.files,
            packages,
        }
    }
}

fn changed_paths(plan: &Plan) -> Vec<String> {
    plan.changed
        .iter()
        .flat_map(|c| std::iter::once(c.path.clone()).chain(c.from.clone()))
        .collect()
}

pub fn replay(replayer: &Replayer, rows: &[Row], window: &Window) -> Replayed {
    let mut out = Replayed::default();
    let mut rows: Vec<&Row> = rows
        .iter()
        .filter(|r| window.contains(&r.created_at))
        .collect();
    rows.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    for row in rows.iter().filter(|r| r.jobs.iter().any(Job::failed)) {
        out.runs += 1;
        replay_row(replayer, row, &rows, &mut out);
    }
    out
}

fn record(
    out: &mut Replayed,
    row: &Row,
    job: &str,
    target: Target,
    outcome: Outcome,
    detail: impl Into<String>,
    changed: &[String],
) {
    out.failures.push(Failure {
        run_id: row.run_id,
        attempt: row.attempt,
        event: row.event.clone(),
        pr: row.pr,
        head_sha: row.head_sha.clone(),
        job: job.to_string(),
        target,
        outcome,
        hit_by: None,
        detail: detail.into(),
        changed: changed.to_vec(),
    });
}

fn replay_row(r: &Replayer, row: &Row, all: &[&Row], out: &mut Replayed) {
    let mut failed: Vec<&Job> = Vec::new();
    for job in row.jobs.iter().filter(|j| j.failed()) {
        if r.sources.ignores(job) {
            record(out, row, &job.name, Target::Job, Outcome::Ignored, "", &[]);
        } else if !r.sources.watches(job) {
            record(
                out,
                row,
                &job.name,
                Target::Job,
                Outcome::Unwatched,
                "",
                &[],
            );
        } else {
            failed.push(job);
        }
    }
    if failed.is_empty() {
        return;
    }
    let unavailable = |out: &mut Replayed, why: &str| {
        for job in &failed {
            record(
                out,
                row,
                &job.name,
                Target::Job,
                Outcome::Unavailable,
                why,
                &[],
            );
        }
    };
    if !has_commit(r.clone, &row.head_sha) {
        return unavailable(out, "head commit not in the clone");
    }
    let Some(base) = row
        .base_sha
        .as_deref()
        .and_then(|b| merge_base(r.clone, b, &row.head_sha))
    else {
        return unavailable(out, "no merge base with the recorded base");
    };
    let PlanAt {
        plan,
        tests,
        seconds,
    } = match r.plan_between(&base, &row.head_sha) {
        Ok(p) => p,
        Err(e) => {
            for job in &failed {
                record(
                    out,
                    row,
                    &job.name,
                    Target::Job,
                    Outcome::Error,
                    e.clone(),
                    &[],
                );
            }
            return;
        }
    };
    out.plans.push(Planned {
        run_id: row.run_id,
        selected: plan.tests.len(),
        total: tests.len(),
        all: plan.all,
        seconds,
    });
    let changed = changed_paths(&plan);
    let snapshot = r.snapshot();
    let repo = Repo {
        files: &snapshot.files,
        packages: &snapshot.packages,
    };
    let root = r.worktree.path.clone();
    let read = move |f: &str| std::fs::read_to_string(root.join(f)).ok();
    // Attribution reads the worktree, so every job is attributed before
    // anything else can touch it.
    let named: Vec<(&Job, Vec<Target>)> = failed
        .into_iter()
        .map(|job| (job, targets(job, &r.sources, &repo, &tests, &read)))
        .collect();
    for (job, found) in named {
        let flaky = passed_on_another_attempt(row, job, all);
        let mut retried = None;
        for target in found {
            let hit_by = in_plan(&plan, &target);
            let outcome = if target == Target::Job {
                Outcome::Unattributed
            } else if flaky {
                Outcome::Flaky
            } else if hit_by.is_some() {
                Outcome::Hit
            } else if *retried
                .get_or_insert_with(|| passed_with_same_change(r, row, &base, job, all))
            {
                Outcome::Unconfirmed
            } else {
                Outcome::Miss
            };
            record(out, row, &job.name, target, outcome, "", &changed);
            if outcome == Outcome::Hit {
                out.failures.last_mut().expect("just recorded").hit_by = hit_by;
            }
        }
    }
}

fn passed_on_another_attempt(row: &Row, job: &Job, all: &[&Row]) -> bool {
    all.iter().any(|other| {
        other.run_id == row.run_id
            && other.attempt != row.attempt
            && other
                .jobs
                .iter()
                .any(|j| j.name == job.name && j.conclusion == "success")
    })
}

/// A later run of the same pull request passed the job with the same change:
/// the same head, or a head whose diff from its merge base has the same
/// patch id (a rebase). The change can't be what failed.
fn passed_with_same_change(r: &Replayer, row: &Row, base: &str, job: &Job, all: &[&Row]) -> bool {
    let Some(pr) = row.pr else {
        return false;
    };
    let mut ours: Option<Option<String>> = None;
    all.iter().any(|other| {
        let passed = other.pr == Some(pr)
            && other.created_at > row.created_at
            && other
                .jobs
                .iter()
                .any(|j| j.name == job.name && j.conclusion == "success");
        if !passed {
            return false;
        }
        if other.head_sha == row.head_sha {
            return true;
        }
        let theirs = other
            .base_sha
            .as_deref()
            .and_then(|b| merge_base(r.clone, b, &other.head_sha))
            .and_then(|mb| patch_id(r.clone, &mb, &other.head_sha));
        let ours = ours.get_or_insert_with(|| patch_id(r.clone, base, &row.head_sha));
        theirs.is_some() && &theirs == ours
    })
}
