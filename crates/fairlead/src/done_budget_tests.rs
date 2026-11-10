use super::*;

fn debt(step: &str, files: usize, why: &str, due: &str) -> Debt {
    Debt {
        step: step.into(),
        files,
        why: why.into(),
        due: due.into(),
    }
}

#[test]
fn classes_order_nearest_first() {
    let mut classes = vec![
        Class::Blanket,
        Class::Chain(4),
        Class::Smoke,
        Class::Chain(3),
        Class::Owner,
        Class::Direct,
        Class::Changed,
    ];
    classes.sort();
    assert_eq!(
        classes,
        vec![
            Class::Changed,
            Class::Direct,
            Class::Owner,
            Class::Smoke,
            Class::Chain(3),
            Class::Chain(4),
            Class::Blanket,
        ]
    );
}

#[test]
fn a_runners_time_per_file_is_the_median_of_its_passing_steps() {
    let event = |seconds: f64, files: u64, passed: bool| {
        serde_json::json!({"stage": "done", "steps": [
            {"id": "unit", "passed": passed, "seconds": seconds, "files": files},
            {"id": "typecheck", "passed": true, "seconds": 30.0},
        ]})
        .to_string()
    };
    let log = [
        event(10.0, 10, true),
        event(40.0, 10, true),
        event(20.0, 10, true),
        // A failed step stops early, so its time says nothing.
        event(1.0, 100, false),
        "{\"stage\":\"write\"}".to_string(),
        "not json".to_string(),
    ]
    .join("\n");
    let per_file = seconds_per_file(&log);
    assert_eq!(per_file.get("unit"), Some(&2.0));
    // A step that names no files has no time per file.
    assert!(!per_file.contains_key("typecheck"));
}

#[test]
fn the_pass_line_sums_what_each_runner_owes() {
    assert_eq!(owed_line(&[]), "");
    let debts = [
        debt("api", 240, "budget", "merge"),
        debt("api", 11, "blanket", "merge"),
        debt("e2e", 119, "blanket", "merge"),
    ];
    assert_eq!(
        owed_line(&debts),
        "; owed to CI before merge: api 251, e2e 119 test files (`fairlead receipt` lists them)"
    );
    assert_eq!(
        owed_lines(&debts),
        vec![
            "api: 251 test files to CI before merge (240 budget, 11 blanket)",
            "e2e: 119 test files to CI before merge (119 blanket)",
        ]
    );
    let nightly = [debt("unit", 5, "untimed", "nightly")];
    assert!(owed_line(&nightly).contains("by the nightly run"));
}
