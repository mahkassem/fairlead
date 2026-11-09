use super::*;

fn job(name: &str, conclusion: &str, steps: &[&str], log: &[&str]) -> Job {
    Job {
        name: name.into(),
        conclusion: conclusion.into(),
        failed_steps: steps.iter().map(|s| s.to_string()).collect(),
        annotations: Vec::new(),
        annotations_capped: false,
        log: log.iter().map(|s| s.to_string()).collect(),
        image: None,
        seconds: None,
    }
}

fn row(run_id: u64, attempt: u32, sha: &str, day: u32, jobs: Vec<Job>) -> Row {
    Row {
        repo: "o/r".into(),
        run_id,
        attempt,
        event: "push".into(),
        workflow: "ci".into(),
        pr: None,
        head_sha: sha.into(),
        base_sha: None,
        created_at: format!("2026-11-{day:02}T10:00:00Z"),
        conclusion: if jobs.iter().any(Job::failed) {
            "failure".into()
        } else {
            "success".into()
        },
        seconds: None,
        jobs,
    }
}

fn vitest() -> Vec<(Regex, Extractor)> {
    vec![(Regex::new("^unit$").unwrap(), Extractor::Vitest)]
}

const A: &str = " FAIL  src/a.test.ts > adds";
const B: &str = " FAIL  src/b.test.ts > subtracts";

fn verdicts(row: &Row, same_tree: &[Row], base: &[Row]) -> Vec<(String, Verdict, String)> {
    let x = vitest();
    let ev = Evidence {
        same_tree,
        base,
        base_branch: "main",
        extractors: &x,
    };
    judge(row, &ev)
        .into_iter()
        .map(|f| (f.test.unwrap_or_else(|| f.job.clone()), f.verdict, f.why))
        .collect()
}

#[test]
fn a_failure_the_base_branch_has_is_inherited_since_its_streak_began() {
    let run = row(
        9,
        1,
        "head",
        8,
        vec![job(
            "unit",
            "failure",
            &["Test"],
            &[A, "Error: boom", B, "AssertionError: expected 1 to be 2"],
        )],
    );
    let base = vec![
        row(
            7,
            1,
            "b7c0ffee000",
            7,
            vec![job("unit", "failure", &["Test"], &[A, "Error: boom"])],
        ),
        row(
            6,
            1,
            "b6c0ffee000",
            6,
            vec![job("unit", "failure", &["Test"], &[A, "Error: boom"])],
        ),
        row(
            5,
            1,
            "b5c0ffee000",
            5,
            vec![job("unit", "success", &[], &[])],
        ),
    ];
    let v = verdicts(&run, &[], &base);
    assert_eq!(v[0].0, "src/a.test.ts > adds");
    assert_eq!(v[0].1, Verdict::Inherited);
    assert_eq!(v[0].2, "red on main since b6c0ffe (2026-11-06)");
    assert_eq!(v[1].1, Verdict::Own);
}

#[test]
fn a_failure_main_has_fixed_since_says_to_update_the_branch() {
    let run = row(
        9,
        1,
        "head",
        8,
        vec![job("unit", "failure", &["Test"], &[A, "Error: boom"])],
    );
    let base = vec![
        row(
            7,
            1,
            "b7c0ffee000",
            7,
            vec![job("unit", "success", &[], &[])],
        ),
        row(
            6,
            1,
            "b6c0ffee000",
            6,
            vec![job("unit", "failure", &["Test"], &[A, "Error: boom"])],
        ),
    ];
    let v = verdicts(&run, &[], &base);
    assert_eq!(v[0].1, Verdict::Inherited);
    assert!(v[0].2.contains("green again at b7c0ffe"), "{}", v[0].2);
}

#[test]
fn a_job_that_passed_on_another_attempt_is_flaky_before_anything_else() {
    let run = row(
        9,
        1,
        "head",
        8,
        vec![job("unit", "failure", &["Test"], &[A, "Error: boom"])],
    );
    let again = row(9, 2, "head", 8, vec![job("unit", "success", &[], &[])]);
    let base = vec![row(
        7,
        1,
        "b7",
        7,
        vec![job("unit", "failure", &["Test"], &[A, "Error: boom"])],
    )];
    let v = verdicts(&run, &[again], &base);
    assert_eq!(v[0].1, Verdict::Flaky);
    let other_run = row(10, 1, "head", 8, vec![job("unit", "success", &[], &[])]);
    assert_eq!(verdicts(&run, &[other_run], &[])[0].1, Verdict::Flaky);
}

#[test]
fn a_timeout_with_no_assertion_is_flaky_and_with_one_is_own() {
    let timed = row(
        9,
        1,
        "head",
        8,
        vec![job(
            "unit",
            "failure",
            &["Test"],
            &[A, "Error: Test timed out in 5000ms."],
        )],
    );
    let v = verdicts(&timed, &[], &[]);
    assert_eq!(
        (v[0].1, v[0].2.as_str()),
        (Verdict::Flaky, "it timed out with no assertion failing")
    );
    let asserted = row(
        9,
        1,
        "head",
        8,
        vec![job(
            "unit",
            "failure",
            &["Test"],
            &[A, "AssertionError: expected 1 to be 2"],
        )],
    );
    assert_eq!(verdicts(&asserted, &[], &[])[0].1, Verdict::Own);
}

#[test]
fn a_job_that_failed_in_setup_or_out_of_memory_is_infra() {
    let setup = row(
        9,
        1,
        "head",
        8,
        vec![job(
            "unit",
            "failure",
            &["Install dependencies"],
            &["npm ERR! 503 Service Unavailable"],
        )],
    );
    let v = verdicts(&setup, &[], &[]);
    assert_eq!(v.len(), 1, "one failure for the job");
    assert_eq!(v[0].0, "unit");
    assert_eq!(v[0].1, Verdict::Infra);
    let oom = row(
        9,
        1,
        "head",
        8,
        vec![job(
            "e2e",
            "failure",
            &["Run"],
            &["##[error]Process completed with exit code 137."],
        )],
    );
    assert_eq!(verdicts(&oom, &[], &[])[0].1, Verdict::Infra);
}

#[test]
fn a_job_failure_with_no_test_is_inherited_only_on_the_same_fingerprint() {
    let run = row(
        9,
        1,
        "head",
        8,
        vec![job(
            "lint",
            "failure",
            &["Lint"],
            &["error: unused import at src/x.rs:3:1"],
        )],
    );
    let same = vec![row(
        7,
        1,
        "b7",
        7,
        vec![job(
            "lint",
            "failure",
            &["Lint"],
            &["error: unused import at src/x.rs:9:4"],
        )],
    )];
    assert_eq!(verdicts(&run, &[], &same)[0].1, Verdict::Inherited);
    let other = vec![row(
        7,
        1,
        "b7",
        7,
        vec![job("lint", "failure", &["Lint"], &["error: missing docs"])],
    )];
    assert_eq!(verdicts(&run, &[], &other)[0].1, Verdict::Own);
}

#[test]
fn annotations_name_tests_without_an_extractor() {
    let mut j = job("e2e", "failure", &["Run"], &["Error: locator not visible"]);
    j.annotations = vec![crate::dataset::Annotation {
        path: "e2e/login.spec.ts".into(),
        title: "signs in".into(),
    }];
    let found = found_in(&j, &[]);
    assert_eq!(
        found[0].test.as_deref(),
        Some("e2e/login.spec.ts > signs in")
    );
    assert_eq!(found[0].message, "Error: locator not visible");
}
