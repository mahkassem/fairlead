//! Replaying under two configs: the one being tried and the base branch's
//! committed one, reported side by side with each failure that moved
//! between hit and miss, so a config change shows what it costs in recall
//! without replaying twice and diffing the reports by hand.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use fairlead_core::config::{self, Config};
use serde::Serialize;

use crate::git::{commit_of, top_level, Worktree};
use crate::report::Report;
use crate::run::{Failure, Outcome, Replayed, Target};

/// The config to compare with, or why there's none.
pub enum Base {
    Found {
        config: Box<Config>,
        /// Where it came from, such as `fairlead.toml at origin/HEAD (1a2b3c4d5)`.
        label: String,
        commit: Option<String>,
    },
    /// No comparison, and the sentence that says why.
    None(Status, String),
}

/// The clone's default branch head: `origin/HEAD` where the clone has
/// one, else the commit its own `HEAD` names.
pub fn default_head(clone: &Path) -> Option<(&'static str, String)> {
    ["origin/HEAD", "HEAD"].into_iter().find_map(|name| {
        let rev = if name == "HEAD" {
            "HEAD".to_string()
        } else {
            format!("refs/remotes/{name}")
        };
        commit_of(clone, &rev).map(|sha| (name, sha))
    })
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}

/// The config committed at the default branch head, found as the planner
/// finds one at the repository's root and loaded as CI loads it: packs and
/// `extends` resolve from that commit's tree, and no local file or
/// `FAIRLEAD_*` variable layers over it. It's read from `worktree`, which
/// is left at that commit.
pub fn base_branch_config(clone: &Path, worktree: &Worktree) -> Base {
    let Some((name, sha)) = default_head(clone) else {
        return Base::None(
            Status::NoBase,
            "the clone has no default branch head to read a config from".into(),
        );
    };
    let at = format!("{name} ({})", short(&sha));
    if top_level(clone, &sha, &config::PROJECT_NAMES).is_empty() {
        return Base::None(
            Status::NoBase,
            format!("no fairlead.toml, .yaml or .yml is committed at {at}"),
        );
    }
    let unusable = |why: String| Base::None(Status::Unusable, why);
    if let Err(e) = worktree.checkout(&sha) {
        return unusable(format!("couldn't check out {at}: {e}"));
    }
    let path = match config::find_config(&worktree.path) {
        Ok(Some(path)) => path,
        Ok(None) => {
            return Base::None(
                Status::NoBase,
                format!("no fairlead.toml, .yaml or .yml is committed at {at}"),
            )
        }
        Err(e) => return unusable(format!("the config at {at} doesn't load: {e}")),
    };
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let label = format!("{file} at {at}");
    match config::load_file(&path, &[]) {
        Err(e) => unusable(format!("{label} doesn't load: {e}")),
        Ok(loaded) if !loaded.problems.is_empty() => {
            let first = &loaded.problems[0];
            unusable(format!(
                "{label} has problems ({}: {}; see `fairlead config check`)",
                first.key, first.message
            ))
        }
        Ok(loaded) => Base::Found {
            config: Box::new(loaded.config),
            label,
            commit: Some(sha),
        },
    }
}

/// Whether two loaded configs plan and judge alike: the planner's own
/// digest, which leaves out the sections that never change a plan, with
/// the pack names and the version floor left out too, since a pack's rules
/// count through the merge.
pub fn same(a: &Config, b: &Config) -> bool {
    let key = |c: &Config| {
        let mut c = c.clone();
        c.extends.clear();
        c.fairlead = None;
        fairlead_tests::digest::config_digest(&c)
    };
    key(a) == key(b)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Both configs were replayed.
    Compared,
    /// The configs plan alike, so one replay is the answer for both.
    Same,
    /// No config is committed at the default branch head.
    NoBase,
    /// One is committed, but it doesn't load or has problems.
    Unusable,
}

/// One side's headline numbers.
#[derive(Debug, Clone, Serialize)]
pub struct Side {
    pub judged: usize,
    pub hits: usize,
    pub misses: usize,
    pub unconfirmed: usize,
    pub recall: Option<f64>,
    pub strict_recall: Option<f64>,
    pub median_selected: Option<f64>,
    pub p90_selected: Option<f64>,
    pub run_all_share: Option<f64>,
}

impl Side {
    pub fn of(r: &Report) -> Side {
        Side {
            judged: r.judged,
            hits: r.hits,
            misses: r.misses.len(),
            unconfirmed: r.unconfirmed,
            recall: r.recall,
            strict_recall: r.strict_recall,
            median_selected: r.median_selected,
            p90_selected: r.p90_selected,
            run_all_share: r.run_all_share,
        }
    }
}

/// How one side judged a failure, and why.
#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    /// `hit`, `miss` or `unconfirmed`.
    pub outcome: String,
    pub why: String,
}

/// A failure one config hits and the other misses.
#[derive(Debug, Clone, Serialize)]
pub struct Moved {
    pub run_id: u64,
    pub attempt: u32,
    pub event: String,
    pub pr: Option<u64>,
    pub head_sha: String,
    pub job: String,
    pub target: String,
    /// `hit` or `miss`, under the base config and under this one.
    pub from: String,
    pub to: String,
    pub base: Verdict,
    pub this: Verdict,
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub status: Status,
    /// Where the base config came from: a file at a commit, or a path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub this: Option<Side>,
    pub moved: Vec<Moved>,
}

impl Comparison {
    /// No comparison, for the reason `note` gives.
    pub fn none(status: Status, note: String) -> Comparison {
        Comparison {
            status,
            base_config: None,
            base_commit: None,
            note,
            base: None,
            this: None,
            moved: Vec::new(),
        }
    }

    pub fn same(label: String, commit: Option<String>) -> Comparison {
        Comparison {
            note: format!("the config plans alike to {label}, so there's nothing to compare"),
            base_config: Some(label),
            base_commit: commit,
            ..Comparison::none(Status::Same, String::new())
        }
    }

    pub fn compared(
        label: String,
        commit: Option<String>,
        base: (&Report, &Replayed),
        this: (&Report, &Replayed),
    ) -> Comparison {
        let moved = moved(base.1, this.1);
        let up = moved.iter().filter(|m| m.to == "hit").count();
        Comparison {
            status: Status::Compared,
            note: format!(
                "against {label}: {} hit to miss, {up} miss to hit",
                moved.len() - up
            ),
            base_config: Some(label),
            base_commit: commit,
            base: Some(Side::of(base.0)),
            this: Some(Side::of(this.0)),
            moved,
        }
    }
}

fn target_name(f: &Failure) -> String {
    match &f.target {
        Target::Test(p) => p.clone(),
        Target::Check(id) => format!("check {id}"),
        Target::Job => format!("job {}", f.job),
    }
}

/// A hit, a miss, or neither: an unconfirmed failure is a miss strict
/// recall counts, so it moves too.
fn side_of(o: Outcome) -> Option<&'static str> {
    match o {
        Outcome::Hit => Some("hit"),
        Outcome::Miss | Outcome::Unconfirmed => Some("miss"),
        _ => None,
    }
}

fn verdict(f: &Failure) -> Verdict {
    let (outcome, why) = match f.outcome {
        Outcome::Hit => ("hit", f.why.clone()),
        Outcome::Unconfirmed => (
            "unconfirmed",
            "not in the plan; a later run of the same change passed the job".into(),
        ),
        _ => ("miss", "not in the plan".into()),
    };
    Verdict {
        outcome: outcome.into(),
        why,
    }
}

/// Each failure judged a hit on one side and a miss on the other, matched
/// by run, attempt, job and target, in the order this side recorded them.
pub fn moved(base: &Replayed, this: &Replayed) -> Vec<Moved> {
    let key = |f: &Failure| (f.run_id, f.attempt, f.job.clone(), target_name(f));
    let before: HashMap<_, &Failure> = base.failures.iter().map(|f| (key(f), f)).collect();
    this.failures
        .iter()
        .filter_map(|now| {
            let was = before.get(&key(now))?;
            let (from, to) = (side_of(was.outcome)?, side_of(now.outcome)?);
            (from != to).then(|| Moved {
                run_id: now.run_id,
                attempt: now.attempt,
                event: now.event.clone(),
                pr: now.pr,
                head_sha: now.head_sha.clone(),
                job: now.job.clone(),
                target: target_name(now),
                from: from.into(),
                to: to.into(),
                base: verdict(was),
                this: verdict(now),
                changed: now.changed.clone(),
            })
        })
        .collect()
}

fn pct(v: Option<f64>) -> String {
    v.map_or("-".into(), |v| format!("{:.1}%", v * 100.0))
}

/// The comparison as the text report's last section.
pub fn text(c: &Comparison) -> String {
    let mut out = String::new();
    let (Some(base), Some(this)) = (&c.base, &c.this) else {
        let _ = writeln!(out, "\n  base config: {}", c.note);
        return out;
    };
    let label = c.base_config.as_deref().unwrap_or("the base config");
    let _ = writeln!(out, "\n  compared with {label}:");
    let _ = writeln!(out, "    {:<16}  {:>8}  {:>8}", "", "base", "this");
    let rows: [(&str, String, String); 7] = [
        ("recall", pct(base.recall), pct(this.recall)),
        (
            "strict recall",
            pct(base.strict_recall),
            pct(this.strict_recall),
        ),
        ("judged", base.judged.to_string(), this.judged.to_string()),
        ("misses", base.misses.to_string(), this.misses.to_string()),
        (
            "selected median",
            pct(base.median_selected),
            pct(this.median_selected),
        ),
        (
            "selected p90",
            pct(base.p90_selected),
            pct(this.p90_selected),
        ),
        ("run_all", pct(base.run_all_share), pct(this.run_all_share)),
    ];
    for (name, b, t) in rows {
        let _ = writeln!(out, "    {name:<16}  {b:>8}  {t:>8}");
    }
    let up = c.moved.iter().filter(|m| m.to == "hit").count();
    let _ = writeln!(
        out,
        "    moved: {} hit to miss, {up} miss to hit",
        c.moved.len() - up
    );
    for m in &c.moved {
        let _ = writeln!(
            out,
            "\n    {} -> {}  run {} attempt {} (PR {})  {}  [{}]  at {}",
            m.from,
            m.to,
            m.run_id,
            m.attempt,
            m.pr.map_or("-".into(), |p| p.to_string()),
            m.target,
            m.job,
            short(&m.head_sha)
        );
        let _ = writeln!(out, "        base: {}", m.base.why);
        let _ = writeln!(out, "        this: {}", m.this.why);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(run: u64, path: &str, outcome: Outcome, why: &str) -> Failure {
        Failure {
            run_id: run,
            attempt: 1,
            event: "pull_request".into(),
            pr: Some(run),
            head_sha: format!("{run:040}"),
            base_sha: None,
            created_at: "2026-09-01T10:00:00Z".into(),
            job: "test".into(),
            target: Target::Test(path.into()),
            outcome,
            hit_by: None,
            why: why.into(),
            judged: None,
            detail: String::new(),
            changed: vec!["src/a.ts".into()],
        }
    }

    fn replayed(failures: Vec<Failure>) -> Replayed {
        Replayed {
            failures,
            ..Replayed::default()
        }
    }

    #[test]
    fn a_failure_moves_only_between_a_hit_and_a_miss_or_unconfirmed() {
        let base = replayed(vec![
            failure(1, "a.test.ts", Outcome::Miss, ""),
            failure(2, "b.test.ts", Outcome::Hit, "selected: changed"),
            failure(3, "c.test.ts", Outcome::Hit, "selected: changed"),
            failure(4, "d.test.ts", Outcome::Error, ""),
            failure(5, "e.test.ts", Outcome::Miss, ""),
        ]);
        let this = replayed(vec![
            failure(
                1,
                "a.test.ts",
                Outcome::Hit,
                "selected: 1 hops from src/a.ts",
            ),
            failure(2, "b.test.ts", Outcome::Unconfirmed, ""),
            failure(3, "c.test.ts", Outcome::Hit, "selected: changed"),
            failure(4, "d.test.ts", Outcome::Hit, "selected: changed"),
            failure(5, "e.test.ts", Outcome::Unconfirmed, ""),
            failure(6, "f.test.ts", Outcome::Hit, "selected: changed"),
        ]);
        let moved = moved(&base, &this);
        let runs: Vec<(u64, &str, &str)> = moved
            .iter()
            .map(|m| (m.run_id, m.from.as_str(), m.to.as_str()))
            .collect();
        assert_eq!(runs, [(1, "miss", "hit"), (2, "hit", "miss")]);
        assert_eq!(moved[0].base.why, "not in the plan");
        assert_eq!(moved[0].this.why, "selected: 1 hops from src/a.ts");
        assert_eq!(moved[1].this.outcome, "unconfirmed");
    }

    #[test]
    fn pack_names_and_the_version_floor_alone_dont_make_configs_differ() {
        let a = Config::default();
        let mut b = Config {
            fairlead: Some("0.1".into()),
            extends: vec!["packs/rules.toml".into()],
            ..Config::default()
        };
        assert!(same(&a, &b));
        b.tests.unreached = config::Unreached::Warn;
        assert!(!same(&a, &b));
        let mut c = Config::default();
        c.guard.baseline = "elsewhere.json".into();
        assert!(same(&a, &c), "guard never changes a plan");
    }

    #[test]
    fn without_two_sides_the_text_is_the_note_alone() {
        let c = Comparison::none(Status::NoBase, "no fairlead.toml here".into());
        assert_eq!(text(&c), "\n  base config: no fairlead.toml here\n");
    }
}
