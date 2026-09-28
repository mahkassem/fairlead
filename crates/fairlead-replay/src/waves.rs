//! Failure waves after a runner image changes. When a floating label such as
//! `ubuntu-latest` moves to a new image, one job can fail for every pull
//! request for days, whatever each changed. Those failures measure the
//! image, not the planner, so they're kept out of recall, hits and misses
//! alike, and listed so they're visible rather than dropped.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use crate::dataset::Row;
use crate::inherited::{judged, touches, unrelated, MIN_PULLS};
use crate::run::{Failure, Outcome, Target};
use crate::window::add_days;

/// How long after an image version first appears its failures can be a wave.
pub const WINDOW_DAYS: i64 = 7;

/// One test failing in one job across unrelated pull requests, all on an
/// image version that job hadn't run on before.
#[derive(Debug, Clone, Serialize)]
pub struct Wave {
    pub job: String,
    pub path: String,
    /// The image before and after, `name version`.
    pub from: String,
    pub to: String,
    /// When the job's failures first ran on the new image.
    pub since: String,
    pub pulls: Vec<u64>,
    pub would_hit: usize,
    pub would_miss: usize,
    pub would_unconfirmed: usize,
}

/// Each failed job's image, by run, attempt and job name.
fn images<'a>(rows: &[&'a Row]) -> HashMap<(u64, u32, &'a str), &'a str> {
    rows.iter()
        .flat_map(|r| {
            r.jobs.iter().filter_map(move |j| {
                j.image
                    .as_deref()
                    .map(|i| ((r.run_id, r.attempt, j.name.as_str()), i))
            })
        })
        .collect()
}

/// Marks wave failures in place and reports each wave.
pub fn apply(failures: &mut [Failure], rows: &[&Row]) -> Vec<Wave> {
    let image_of = images(rows);
    // Each job's images in the order it first ran on them.
    let mut seen: BTreeMap<&str, Vec<(String, &str)>> = BTreeMap::new();
    let mut ordered: Vec<&&Row> = rows.iter().collect();
    ordered.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    for row in ordered {
        for job in &row.jobs {
            let Some(image) = job.image.as_deref() else {
                continue;
            };
            let list = seen.entry(job.name.as_str()).or_default();
            if !list.iter().any(|(_, i)| *i == image) {
                list.push((row.created_at.clone(), image));
            }
        }
    }
    let mut groups: BTreeMap<(String, String, String), Vec<usize>> = BTreeMap::new();
    for (i, f) in failures.iter().enumerate() {
        let Target::Test(path) = &f.target else {
            continue;
        };
        if f.pr.is_none() || f.event == "push" || !judged(f.outcome) {
            continue;
        }
        if let Some(image) = image_of.get(&(f.run_id, f.attempt, f.job.as_str())) {
            groups
                .entry((f.job.clone(), path.clone(), image.to_string()))
                .or_default()
                .push(i);
        }
    }
    let mut waves = Vec::new();
    for ((job, path, image), members) in groups {
        let Some(list) = seen.get(job.as_str()) else {
            continue;
        };
        let Some(at) = list.iter().position(|(_, i)| *i == image) else {
            continue;
        };
        // The first image a job was seen on isn't a change.
        if at == 0 {
            continue;
        }
        let (since, _) = &list[at];
        let (_, before) = &list[at - 1];
        let until = add_days(&since[..10], WINDOW_DAYS).unwrap_or_default();
        let members: Vec<usize> = members
            .into_iter()
            .filter(|&i| failures[i].created_at[..10] <= *until)
            .filter(|&i| !failures[i].changed.iter().any(|c| touches(c, &path)))
            .collect();
        // A test that already failed there on an older image isn't new with this one.
        let failed_before = failures.iter().any(|f| {
            f.job == job
                && matches!(&f.target, Target::Test(p) if *p == path)
                && image_of
                    .get(&(f.run_id, f.attempt, f.job.as_str()))
                    .is_some_and(|i| list.iter().position(|(_, x)| x == i) < Some(at))
        });
        if failed_before || unrelated(failures, &members) < MIN_PULLS {
            continue;
        }
        let pulls: BTreeSet<u64> = members.iter().filter_map(|&i| failures[i].pr).collect();
        let mut wave = Wave {
            job,
            path,
            from: before.to_string(),
            to: image,
            since: since.clone(),
            pulls: pulls.into_iter().collect(),
            would_hit: 0,
            would_miss: 0,
            would_unconfirmed: 0,
        };
        for &i in &members {
            let f = &mut failures[i];
            match f.outcome {
                Outcome::Hit => wave.would_hit += 1,
                Outcome::Miss => wave.would_miss += 1,
                _ => wave.would_unconfirmed += 1,
            }
            f.judged = Some(f.outcome);
            f.outcome = Outcome::Environment;
        }
        waves.push(wave);
    }
    waves
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Job;

    fn row(run: u64, day: u32, image: &str) -> Row {
        Row {
            repo: "o/r".into(),
            run_id: run,
            attempt: 1,
            event: "pull_request".into(),
            workflow: "ci".into(),
            pr: Some(run),
            head_sha: format!("h{run}"),
            base_sha: Some(format!("b{run}")),
            created_at: format!("2026-09-{day:02}T10:00:00Z"),
            conclusion: "failure".into(),
            jobs: vec![Job {
                name: "unit".into(),
                conclusion: "failure".into(),
                failed_steps: Vec::new(),
                annotations: Vec::new(),
                annotations_capped: false,
                log: Vec::new(),
                image: Some(image.into()),
            }],
        }
    }

    fn failure(run: u64, day: u32, path: &str, changed: &str, outcome: Outcome) -> Failure {
        Failure {
            run_id: run,
            attempt: 1,
            event: "pull_request".into(),
            pr: Some(run),
            head_sha: format!("h{run}"),
            base_sha: Some(format!("b{run}")),
            created_at: format!("2026-09-{day:02}T10:00:00Z"),
            job: "unit".into(),
            target: Target::Test(path.into()),
            outcome,
            hit_by: None,
            judged: None,
            detail: String::new(),
            changed: vec![changed.into()],
        }
    }

    const OLD: &str = "ubuntu-24.04 20260907.1";
    const NEW: &str = "ubuntu-26.04 20260919.1";

    #[test]
    fn unrelated_pulls_failing_a_test_after_the_image_changed_are_a_wave() {
        let rows = [
            row(1, 1, OLD),
            row(2, 20, NEW),
            row(3, 21, NEW),
            row(4, 22, NEW),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let mut fs = vec![
            failure(1, 1, "test/other.test.ts", "src/z.ts", Outcome::Miss),
            failure(2, 20, "test/fs.test.ts", "src/a.ts", Outcome::Miss),
            failure(3, 21, "test/fs.test.ts", "src/b.ts", Outcome::Hit),
            failure(4, 22, "test/fs.test.ts", "src/c.ts", Outcome::Miss),
        ];
        let waves = apply(&mut fs, &refs);
        assert_eq!(waves.len(), 1);
        let w = &waves[0];
        assert_eq!((w.from.as_str(), w.to.as_str()), (OLD, NEW));
        assert_eq!(
            (w.pulls.clone(), w.would_hit, w.would_miss),
            (vec![2, 3, 4], 1, 2)
        );
        assert_eq!(
            fs[0].outcome,
            Outcome::Miss,
            "the older image's failure stays"
        );
        assert!(fs[1..].iter().all(|f| f.outcome == Outcome::Environment));
    }

    #[test]
    fn a_test_that_already_failed_on_the_old_image_or_too_few_pulls_is_no_wave() {
        let rows = [
            row(1, 1, OLD),
            row(2, 20, NEW),
            row(3, 21, NEW),
            row(4, 22, NEW),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let mut regression = vec![
            failure(1, 1, "test/fs.test.ts", "src/z.ts", Outcome::Miss),
            failure(2, 20, "test/fs.test.ts", "src/a.ts", Outcome::Miss),
            failure(3, 21, "test/fs.test.ts", "src/b.ts", Outcome::Miss),
            failure(4, 22, "test/fs.test.ts", "src/c.ts", Outcome::Miss),
        ];
        assert!(apply(&mut regression, &refs).is_empty());
        let mut two = vec![
            failure(1, 1, "test/other.test.ts", "src/z.ts", Outcome::Miss),
            failure(2, 20, "test/fs.test.ts", "src/a.ts", Outcome::Miss),
            failure(3, 21, "test/fs.test.ts", "src/b.ts", Outcome::Miss),
        ];
        assert!(apply(&mut two, &refs).is_empty());
    }

    #[test]
    fn failures_on_the_first_image_seen_or_long_after_the_change_are_no_wave() {
        let rows = [row(2, 1, NEW), row(3, 2, NEW), row(4, 3, NEW)];
        let refs: Vec<&Row> = rows.iter().collect();
        let mut first = vec![
            failure(2, 1, "test/fs.test.ts", "src/a.ts", Outcome::Miss),
            failure(3, 2, "test/fs.test.ts", "src/b.ts", Outcome::Miss),
            failure(4, 3, "test/fs.test.ts", "src/c.ts", Outcome::Miss),
        ];
        assert!(apply(&mut first, &refs).is_empty(), "no change was seen");
        let rows = [
            row(1, 1, OLD),
            row(2, 2, NEW),
            row(3, 20, NEW),
            row(4, 21, NEW),
            row(5, 22, NEW),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let mut late = vec![
            failure(2, 2, "test/other.test.ts", "src/z.ts", Outcome::Miss),
            failure(3, 20, "test/fs.test.ts", "src/a.ts", Outcome::Miss),
            failure(4, 21, "test/fs.test.ts", "src/b.ts", Outcome::Miss),
            failure(5, 22, "test/fs.test.ts", "src/c.ts", Outcome::Miss),
        ];
        assert!(
            apply(&mut late, &refs).is_empty(),
            "past the window from the change"
        );
    }

    #[test]
    fn the_runner_image_comes_from_the_log_header() {
        let log = "2026-09-20T10:00:00.0Z Current runner version: '2.330.0'\n2026-09-20T10:00:00.0Z ##[group]Runner Image\n2026-09-20T10:00:00.0Z Image: ubuntu-24.04\n2026-09-20T10:00:00.0Z Version: 20260907.1.0\n";
        assert_eq!(
            crate::dataset::runner_image(log).as_deref(),
            Some("ubuntu-24.04 20260907.1.0")
        );
        assert_eq!(crate::dataset::runner_image("no header\n"), None);
    }
}
