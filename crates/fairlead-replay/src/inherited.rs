//! Failures a pull request inherited from its base branch. When unrelated
//! pull requests on one base fail the same test in the same job, the base is
//! the likelier cause than any of their changes, so those failures are kept
//! out of adjusted recall, hits and misses alike, with what each would have
//! been. A push run of the base itself failing the test is direct proof.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::dataset::Row;
use crate::run::{Failure, Outcome, Target};

/// Distinct pull requests on one base that must share a failure before it
/// counts as inherited without the base's own run to prove it.
pub const MIN_PULLS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Evidence {
    /// A push run of the base commit failed the test in the same job.
    Base,
    /// Enough unrelated pull requests on the base failed it alike.
    Pulls,
}

/// One test failing in one job across pull requests on one base.
#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub base_sha: String,
    pub job: String,
    pub path: String,
    pub pulls: Vec<u64>,
    pub evidence: Evidence,
    /// A later run on another base passed the job, so the base was fixed.
    pub resolved: bool,
    pub would_hit: usize,
    pub would_miss: usize,
    pub would_unconfirmed: usize,
}

pub(crate) fn judged(outcome: Outcome) -> bool {
    matches!(outcome, Outcome::Hit | Outcome::Miss | Outcome::Unconfirmed)
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Whether a change may be what broke `test`: the test itself, or a file
/// beside it that isn't another test, such as a fixture or a helper. A
/// sibling test can't break it.
pub(crate) fn touches(changed: &str, test: &str) -> bool {
    let name = changed.rsplit('/').next().unwrap_or(changed);
    let sibling_test = name.contains(".test.") || name.contains(".spec.");
    changed == test
        || (!dir_of(test).is_empty() && dir_of(changed) == dir_of(test) && !sibling_test)
}

/// Marks inherited failures in place and reports each group.
pub fn apply(failures: &mut [Failure], rows: &[&Row]) -> Vec<Group> {
    let mut by_key: BTreeMap<(String, String, String), Vec<usize>> = BTreeMap::new();
    for (i, f) in failures.iter().enumerate() {
        let (Target::Test(path), Some(base), true) = (&f.target, &f.base_sha, f.pr.is_some())
        else {
            continue;
        };
        if f.event != "push" && judged(f.outcome) {
            by_key
                .entry((base.clone(), f.job.clone(), path.clone()))
                .or_default()
                .push(i);
        }
    }
    let mut groups = Vec::new();
    for ((base, job, path), members) in by_key {
        let members: Vec<usize> = members
            .into_iter()
            .filter(|&i| !failures[i].changed.iter().any(|c| touches(c, &path)))
            .collect();
        let pulls: BTreeSet<u64> = members.iter().filter_map(|&i| failures[i].pr).collect();
        let proven = failures.iter().any(|f| {
            f.event == "push"
                && f.head_sha == base
                && f.job == job
                && matches!(&f.target, Target::Test(p) if *p == path)
        });
        let evidence = if proven {
            Evidence::Base
        } else if unrelated(failures, &members) >= MIN_PULLS {
            Evidence::Pulls
        } else {
            continue;
        };
        let last = members
            .iter()
            .map(|&i| failures[i].created_at.as_str())
            .max()
            .unwrap_or("");
        let resolved = rows.iter().any(|r| {
            r.created_at.as_str() > last
                && r.base_sha.as_deref().is_some_and(|b| b != base)
                && r.jobs
                    .iter()
                    .any(|j| j.name == job && j.conclusion == "success")
        });
        let mut group = Group {
            base_sha: base,
            job,
            path,
            pulls: pulls.into_iter().collect(),
            evidence,
            resolved,
            would_hit: 0,
            would_miss: 0,
            would_unconfirmed: 0,
        };
        for &i in &members {
            let f = &mut failures[i];
            match f.outcome {
                Outcome::Hit => group.would_hit += 1,
                Outcome::Miss => group.would_miss += 1,
                _ => group.would_unconfirmed += 1,
            }
            f.judged = Some(f.outcome);
            f.outcome = Outcome::Inherited;
        }
        groups.push(group);
    }
    groups
}

/// How many of the pull requests changed no file in common with each other,
/// taken in order: a stack of related changes shares a cause, so only the
/// unrelated ones are evidence about the base.
pub(crate) fn unrelated(failures: &[Failure], members: &[usize]) -> usize {
    let mut by_pull: BTreeMap<u64, BTreeSet<&str>> = BTreeMap::new();
    for &i in members {
        if let Some(pr) = failures[i].pr {
            by_pull
                .entry(pr)
                .or_default()
                .extend(failures[i].changed.iter().map(String::as_str));
        }
    }
    let mut kept: Vec<&BTreeSet<&str>> = Vec::new();
    for set in by_pull.values() {
        if kept.iter().all(|k| k.is_disjoint(set)) {
            kept.push(set);
        }
    }
    kept.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(pr: u64, changed: &[&str], outcome: Outcome) -> Failure {
        Failure {
            run_id: pr,
            attempt: 1,
            event: "pull_request".into(),
            pr: Some(pr),
            head_sha: format!("head{pr}"),
            base_sha: Some("base1".into()),
            created_at: format!("2026-09-{:02}T10:00:00Z", pr),
            job: "e2e".into(),
            target: Target::Test("test/e2e/leak.test.ts".into()),
            outcome,
            hit_by: None,
            judged: None,
            detail: String::new(),
            changed: changed.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn three() -> Vec<Failure> {
        vec![
            failure(1, &["src/watch.ts"], Outcome::Miss),
            failure(2, &["src/config.ts"], Outcome::Miss),
            failure(3, &["src/env.ts"], Outcome::Hit),
        ]
    }

    #[test]
    fn unrelated_pulls_on_one_base_failing_alike_are_inherited_hits_and_misses_alike() {
        let mut fs = three();
        let groups = apply(&mut fs, &[]);
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(
            (g.evidence, g.pulls.clone(), g.would_miss, g.would_hit),
            (Evidence::Pulls, vec![1, 2, 3], 2, 1)
        );
        assert!(!g.resolved, "nothing passed the job later");
        assert!(fs.iter().all(|f| f.outcome == Outcome::Inherited));
        assert_eq!(fs[2].judged, Some(Outcome::Hit));
    }

    #[test]
    fn one_related_pair_among_enough_unrelated_pulls_still_shows_the_base() {
        let mut fs = three();
        fs.push(failure(4, &["src/watch.ts"], Outcome::Miss));
        fs.push(failure(5, &["test/e2e/other.test.ts"], Outcome::Miss));
        let groups = apply(&mut fs, &[]);
        assert_eq!(
            groups[0].pulls,
            [1, 2, 3, 4, 5],
            "a sibling test isn't a cause, and the pair doesn't sink the group"
        );
    }

    #[test]
    fn two_pulls_overlapping_changes_or_a_change_beside_the_test_do_not_qualify() {
        let mut two = three();
        two.pop();
        assert!(apply(&mut two, &[]).is_empty());
        let mut stacked = three();
        stacked[1].changed.push("src/watch.ts".into());
        assert!(
            apply(&mut stacked, &[]).is_empty(),
            "a stack shares a cause"
        );
        let mut beside = three();
        beside[0].changed = vec!["test/e2e/helpers.ts".into()];
        assert!(
            apply(&mut beside, &[]).is_empty(),
            "one pull request touched the test's directory, leaving two"
        );
        assert_eq!(beside[1].outcome, Outcome::Miss);
    }

    #[test]
    fn the_bases_own_push_failing_the_test_proves_it_for_a_single_pull() {
        let mut push = failure(9, &[], Outcome::Miss);
        push.event = "push".into();
        push.pr = None;
        push.head_sha = "base1".into();
        let mut fs = vec![failure(1, &["src/watch.ts"], Outcome::Miss), push];
        let groups = apply(&mut fs, &[]);
        assert_eq!(groups[0].evidence, Evidence::Base);
        assert_eq!(fs[0].outcome, Outcome::Inherited);
        assert_eq!(
            fs[1].outcome,
            Outcome::Miss,
            "the push run is judged on its own"
        );
    }

    #[test]
    fn a_later_run_on_another_base_passing_the_job_resolves_the_group() {
        let later = Row {
            repo: "o/r".into(),
            run_id: 50,
            attempt: 1,
            event: "pull_request".into(),
            workflow: "ci".into(),
            pr: Some(50),
            head_sha: "h50".into(),
            base_sha: Some("base2".into()),
            created_at: "2026-09-20T10:00:00Z".into(),
            conclusion: "success".into(),
            jobs: vec![crate::dataset::Job {
                name: "e2e".into(),
                conclusion: "success".into(),
                failed_steps: Vec::new(),
                annotations: Vec::new(),
                annotations_capped: false,
                log: Vec::new(),
                image: None,
            }],
        };
        let mut fs = three();
        assert!(apply(&mut fs, &[&later])[0].resolved);
    }

    #[test]
    fn the_report_keeps_inherited_failures_out_of_adjusted_recall_and_lists_the_group() {
        let mut fs = three();
        let inherited = apply(&mut fs, &[]);
        let replayed = crate::run::Replayed {
            failures: fs,
            inherited,
            runs: 3,
            ..Default::default()
        };
        let window = crate::window::Window::ending("2026-09-30", 30).unwrap();
        let r = crate::report::report("o/r", &window, 30, &replayed);
        assert_eq!((r.judged, r.recall, r.inherited), (0, None, 3));
        assert_eq!((r.raw_judged, r.raw_recall), (3, Some(1.0 / 3.0)));
        let text = crate::report::text(&r);
        assert!(
            text.contains("inherited test/e2e/leak.test.ts  [e2e]  base base1: unrelated pull requests failed it alike; PRs 1, 2, 3; 1 would-be hits, 2 misses; unresolved"),
            "{text}"
        );
    }
}
