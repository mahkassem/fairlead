use super::*;
use fairlead_replay::dataset::Job;

fn config() -> Config {
    toml::from_str(
        r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "{files}"]

[[checks]]
id = "lint"
command = ["sh"]
"#,
    )
    .unwrap()
}

fn done_event(steps: &[(&str, bool, f64)], plan: f64) -> String {
    let steps: Vec<Value> = steps
        .iter()
        .map(|(id, passed, seconds)| serde_json::json!({"id": id, "passed": passed, "seconds": seconds}))
        .collect();
    serde_json::json!({"at": "2026-11-05T10:00:00Z", "stage": "done", "steps": steps, "plan_seconds": plan})
        .to_string()
}

fn kinds(s: &Speed) -> Vec<(&str, String)> {
    s.lines.iter().map(|l| (l.kind, l.subject.clone())).collect()
}

#[test]
fn a_step_that_never_fails_one_that_always_fails_and_one_faster_than_its_plan_are_named() {
    let mut log = String::new();
    for i in 0..NEVER_FAILED {
        log.push_str(&done_event(&[("unit", true, 0.4), ("lint", true, 20.0 + i as f64), ("guard", false, 1.0)], 2.0));
        log.push('\n');
    }
    log.push_str("{\"at\": \"2026-10-01T00:00:00Z\", \"stage\": \"done\", \"steps\": [{\"id\": \"unit\", \"passed\": false, \"seconds\": 9}]}\n");
    let mut inputs = Inputs::default();
    inputs.add_events(&log, "2026-11-01");
    let s = coach(&inputs, &config(), None);
    assert_eq!(s.plan_seconds, Some(2.0));
    let k = kinds(&s);
    assert_eq!(k[0], ("slowest", "lint (done)".to_string()));
    assert!(k.contains(&("never-failed", "lint (done)".into())), "{k:?}");
    assert!(k.contains(&("never-failed", "unit (done)".into())), "an event before --since doesn't count: {k:?}");
    assert!(k.contains(&("always-fails", "guard (done)".into())), "{k:?}");
    assert!(k.contains(&("run-whole", "unit (done)".into())), "{k:?}");
    assert!(!k.contains(&("never-failed", "guard (done)".into())), "guard isn't a runner or check");
    let whole = s.lines.iter().find(|l| l.kind == "run-whole").unwrap();
    assert_eq!(whole.numbers, "its longest run took 0.4 s against 2.0 s to plan");
    assert!(whole.suggestion.contains("all_from = \"draft\""));
    let later = s.lines.iter().find(|l| l.kind == "never-failed" && l.subject.starts_with("lint")).unwrap();
    assert!(later.suggestion.contains("`[[checks]] id = \"lint\", from = \"merge\"`"), "{}", later.suggestion);
}

#[test]
fn without_a_plan_time_no_run_whole_line_and_the_replay_plan_time_stands_in() {
    let mut inputs = Inputs::default();
    inputs.add_results(&serde_json::json!({"invocations": [
        {"id": "unit", "passed": true, "seconds": 0.5},
        {"id": "unit", "passed": true, "seconds": 0.6},
        {"id": "unit", "passed": true, "seconds": 0.7}]}));
    let none = coach(&inputs, &config(), None);
    assert!(none.lines.iter().all(|l| l.kind != "run-whole"));
    assert!(render(&none).contains("no plan time recorded yet"));
    let replayed = coach(&inputs, &config(), Some(1.5));
    let whole = replayed.lines.iter().find(|l| l.kind == "run-whole").unwrap();
    assert_eq!(whole.subject, "unit (ci run)");
    assert!(replayed.lines[0].suggestion.contains("--shards 2"));
}

fn row(run_id: u64, attempt: u32, pr: u64, conclusion: &str, job_seconds: Option<u64>) -> Row {
    Row {
        repo: "o/r".into(),
        run_id,
        attempt,
        event: "pull_request".into(),
        workflow: "ci".into(),
        pr: Some(pr),
        head_sha: "h".into(),
        base_sha: None,
        created_at: "2026-11-05T10:00:00Z".into(),
        conclusion: conclusion.into(),
        seconds: None,
        jobs: match conclusion {
            "cancelled" => Vec::new(),
            c => vec![Job {
                name: "test".into(),
                conclusion: c.into(),
                failed_steps: Vec::new(),
                annotations: Vec::new(),
                annotations_capped: false,
                log: Vec::new(),
                image: None,
                seconds: job_seconds,
            }],
        },
    }
}

#[test]
fn a_dataset_gives_reruns_cancellations_and_the_pull_requests_behind_them() {
    let mut inputs = Inputs::default();
    inputs.add_rows(vec![
        row(1, 1, 7, "failure", Some(60)),
        row(1, 2, 7, "success", Some(50)),
        row(2, 1, 7, "cancelled", None),
        row(3, 1, 8, "success", Some(40)),
        row(4, 1, 8, "cancelled", None),
    ]);
    let s = coach(&inputs, &config(), None);
    let line = |kind: &str| s.lines.iter().find(|l| l.kind == kind).unwrap().numbers.clone();
    assert_eq!(line("reruns"), "1 reruns over 4 runs; 1 passed only on a rerun of the same commit (25%)");
    assert_eq!(line("cancelled"), "2 of 4 runs cancelled (50%)");
    assert_eq!(line("pulls"), "#7: 2 runs, 1 reruns, 1 cancelled; #8: 2 runs, 0 reruns, 1 cancelled");
    let job = s.steps.iter().find(|st| st.source == Source::Job).unwrap();
    assert_eq!((job.runs, job.failed, job.max), (3, 1, 60.0));
}

#[test]
fn nothing_recorded_says_where_numbers_come_from() {
    let s = coach(&Inputs::default(), &config(), None);
    assert!(render(&s).contains("nothing recorded yet"));
}
