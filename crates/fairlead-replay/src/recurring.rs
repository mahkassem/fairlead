//! Tests that fail alike across unrelated pull requests on different bases.
//! When one test fails in one job for pull requests that share no changed
//! file, none of them touching it, no one change is the likelier cause: the
//! test is, most often by being flaky. Those failures are kept out of
//! adjusted recall, hits and misses alike, and listed with a quarantine
//! entry for a person to confirm.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::inherited::{judged, touches, unrelated, MIN_PULLS};
use crate::run::{Failure, Outcome, Target};

/// One test failing in one job across unrelated pull requests.
#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub job: String,
    pub path: String,
    pub pulls: Vec<u64>,
    pub would_hit: usize,
    pub would_miss: usize,
    pub would_unconfirmed: usize,
}

/// Marks recurring failures in place and reports each group. Runs after the
/// inherited and wave passes, so a failure they explain isn't counted twice.
pub fn apply(failures: &mut [Failure]) -> Vec<Group> {
    let mut by_key: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, f) in failures.iter().enumerate() {
        let Target::Test(path) = &f.target else {
            continue;
        };
        if f.pr.is_some() && f.event != "push" && judged(f.outcome) {
            by_key
                .entry((f.job.clone(), path.clone()))
                .or_default()
                .push(i);
        }
    }
    let mut groups = Vec::new();
    for ((job, path), members) in by_key {
        let members: Vec<usize> = members
            .into_iter()
            .filter(|&i| !failures[i].changed.iter().any(|c| touches(c, &path)))
            .collect();
        if unrelated(failures, &members) < MIN_PULLS {
            continue;
        }
        let pulls: BTreeSet<u64> = members.iter().filter_map(|&i| failures[i].pr).collect();
        let mut group = Group {
            job,
            path,
            pulls: pulls.into_iter().collect(),
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
            f.outcome = Outcome::Recurring;
        }
        groups.push(group);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(pr: u64, base: &str, changed: &[&str], outcome: Outcome) -> Failure {
        Failure {
            run_id: pr,
            attempt: 1,
            event: "pull_request".into(),
            pr: Some(pr),
            head_sha: format!("head{pr}"),
            base_sha: Some(base.into()),
            created_at: format!("2026-09-{:02}T10:00:00Z", pr),
            job: "jvm".into(),
            target: Target::Test("okhttp/src/jvmTest/kotlin/okhttp3/DuplexTest.kt".into()),
            outcome,
            hit_by: None,
            judged: None,
            detail: String::new(),
            changed: changed.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn one_test_failing_for_three_unrelated_pulls_on_different_bases_recurs() {
        let mut fs = vec![
            failure(
                1,
                "b1",
                &["okhttp-dnsoverhttps/src/main/kotlin/Dns.kt"],
                Outcome::Miss,
            ),
            failure(
                2,
                "b2",
                &["okhttp/src/jvmTest/kotlin/okhttp3/FastFallbackTest.kt"],
                Outcome::Miss,
            ),
            failure(
                3,
                "b3",
                &["okhttp/src/main/kotlin/okhttp3/Call.kt"],
                Outcome::Hit,
            ),
        ];
        let groups = apply(&mut fs);
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(
            (g.pulls.clone(), g.would_miss, g.would_hit),
            (vec![1, 2, 3], 2, 1)
        );
        assert!(fs.iter().all(|f| f.outcome == Outcome::Recurring));
        assert_eq!(fs[2].judged, Some(Outcome::Hit), "a hit is set aside alike");
    }

    #[test]
    fn related_pulls_or_a_pull_touching_the_test_are_not_evidence() {
        let mut fs = vec![
            failure(1, "b1", &["src/a.kt"], Outcome::Miss),
            failure(2, "b2", &["src/a.kt", "src/b.kt"], Outcome::Miss),
            failure(
                3,
                "b3",
                &["okhttp/src/jvmTest/kotlin/okhttp3/DuplexTest.kt"],
                Outcome::Miss,
            ),
        ];
        assert!(apply(&mut fs).is_empty());
        assert!(fs.iter().all(|f| f.outcome == Outcome::Miss));
    }
}
