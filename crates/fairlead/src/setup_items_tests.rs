use super::*;
use crate::setup_prove::{Proof, Record};

fn config(text: &str) -> Config {
    toml::from_str(text).unwrap()
}

fn suites(files: Vec<Vec<&str>>) -> Suites {
    let files: Vec<Vec<String>> = files
        .into_iter()
        .map(|f| f.into_iter().map(String::from).collect())
        .collect();
    Suites {
        tests: files.iter().map(Vec::len).sum(),
        files,
        unmatched: Vec::new(),
        ambiguous: Vec::new(),
        error: None,
    }
}

const TWO: &str = r#"
[[tests.runners]]
id = "unit"
match = ["src/**"]
command = ["npx", "vitest", "run", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["npx", "playwright", "test", "{files}"]
"#;

#[test]
fn only_the_track_decides_whether_ci_is_required_and_where_it_prints() {
    for id in ["binary", "config", "runners", "hooks", "git-hook", "agents"] {
        assert!(required(id, Track::New), "{id}");
    }
    for id in ["proved", "smoke", "gate", "measure", "tests"] {
        assert!(!required(id, Track::Ongoing), "{id}");
    }
    assert!(required("ci", Track::Ongoing));
    assert!(!required("ci", Track::Mvp));
    let list = || vec![ok("hooks", ""), ok("ci", ""), ok("gate", "")];
    let ids = |items: Vec<Item>| items.iter().map(|i| i.id).collect::<Vec<_>>();
    assert_eq!(ids(in_order(list(), Track::Mvp)), ["hooks", "ci", "gate"]);
    assert_eq!(ids(in_order(list(), Track::New)), ["hooks", "gate", "ci"]);
}

#[test]
fn open_counts_every_open_item_and_only_a_required_one_blocks() {
    let mut items = vec![
        ok("binary", ""),
        open("gate", "", "fairlead done"),
        later("ci", ""),
        open("hooks", "", "fairlead hooks install"),
    ];
    assert_eq!(open_of(&items), (2, Some("gate")));
    assert!(!blocked(&items));
    items[3].required = true;
    assert!(blocked(&items));
    assert_eq!(open_of(&items[..1]), (0, None));
}

#[test]
fn runners_are_open_for_a_test_with_no_runner_or_two_or_a_command_naming_no_file() {
    let c = config(TWO);
    let fine = suites(vec![vec!["src/a.test.ts"], vec!["e2e/b.spec.ts"]]);
    let item = runners(&c, &fine);
    assert_eq!(item.state, State::Ok);
    assert_eq!(
        item.text,
        "2 runners, 2 test files, each with exactly one runner"
    );
    let unmatched = Suites {
        unmatched: vec!["lib/x.test.ts".into()],
        ..suites(vec![vec![], vec![]])
    };
    let item = runners(&c, &unmatched);
    assert_eq!(item.text, "1 test file with no runner, first lib/x.test.ts");
    assert!(item.fix.unwrap().contains("`lib/x.test.ts`"));
    let twice = Suites {
        ambiguous: vec![("src/a.spec.ts".into(), vec!["unit".into(), "e2e".into()])],
        ..suites(vec![vec!["src/a.spec.ts"], vec![]])
    };
    let item = runners(&c, &twice);
    assert_eq!(
        item.text,
        "1 test file with more than one runner, first src/a.spec.ts (unit, e2e)"
    );
    let whole = config(
        "[[tests.runners]]\nid = \"whole\"\nmatch = [\"e2e/**\"]\ncommand = [\"npx\", \"playwright\", \"test\"]\n",
    );
    let item = runners(&whole, &suites(vec![vec!["e2e/b.spec.ts"]]));
    assert_eq!(item.state, State::Open);
    assert_eq!(
        item.fix.as_deref(),
        Some("put `{files}` where the test files go in fairlead.toml")
    );
    let none = runners(&Config::default(), &suites(vec![]));
    assert_eq!(none.state, State::Ok);
    let unrun = Suites {
        tests: 3,
        ..suites(vec![])
    };
    assert_eq!(runners(&Config::default(), &unrun).state, State::Open);
}

#[test]
fn proved_reads_the_record_as_open_later_or_ok() {
    let c = config(TWO);
    let s = suites(vec![vec!["src/a.test.ts"], vec!["e2e/b.spec.ts"]]);
    let proof = |runner: &str, passed: Option<bool>, why: Option<&str>| Proof {
        runner: runner.into(),
        file: "f".into(),
        passed,
        seconds: 0.4,
        why: why.map(String::from),
    };
    let none = proved(&c, &s, &Record::default());
    assert_eq!(none.state, State::Open);
    assert_eq!(
        none.text,
        "2 of 2 runners haven't run a test file here: unit, e2e"
    );
    let later = Record {
        proved: vec![
            proof("unit", Some(true), None),
            proof("e2e", None, Some("need \"browser\" not met")),
        ],
    };
    let item = proved(&c, &s, &later);
    assert_eq!(item.state, State::Later);
    assert_eq!(item.text, "e2e can't run here: need \"browser\" not met");
    let failed = Record {
        proved: vec![
            proof("unit", Some(false), Some("1 test failed (exit 1)")),
            proof("e2e", None, Some("need \"browser\" not met")),
        ],
    };
    let item = proved(&c, &s, &failed);
    assert_eq!(item.state, State::Open);
    assert_eq!(item.text, "unit failed on f: 1 test failed (exit 1)");
    let passed = Record {
        proved: vec![
            proof("unit", Some(true), None),
            proof("e2e", Some(true), None),
        ],
    };
    let item = proved(&c, &s, &passed);
    assert_eq!(item.state, State::Ok);
    assert_eq!(
        item.text,
        "2 runners ran one test file: unit 0.4 s, e2e 0.4 s"
    );
    let empty = proved(&c, &suites(vec![vec![], vec![]]), &passed);
    assert_eq!(empty.state, State::Later);
}

#[test]
fn smoke_is_wanted_past_50_test_files_or_behind_a_need() {
    let c = config(TWO);
    let many: Vec<String> = (0..51).map(|i| format!("src/{i}.test.ts")).collect();
    let big = suites(vec![many.iter().map(String::as_str).collect(), vec![]]);
    let item = smoke(&c, &big);
    assert_eq!(item.state, State::Open);
    assert_eq!(item.text, "unit: 51 test files, and it names no `smoke`");
    let small = suites(vec![vec!["src/a.test.ts"], vec!["e2e/b.spec.ts"]]);
    assert_eq!(smoke(&c, &small).state, State::Ok);
    let needy = config(&format!(
        "{TWO}\n[[needs]]\nid = \"browser\"\nenv = \"BROWSER\"\ntests = [\"e2e/**\"]\n"
    ));
    let item = smoke(&needy, &small);
    assert_eq!(
        item.text,
        "e2e: need \"browser\" covers it, and it names no `smoke`"
    );
    let named = config(&format!(
        "{}smoke = [\"e2e/b.spec.ts\"]\n\n[[needs]]\nid = \"browser\"\nenv = \"BROWSER\"\ntests = [\"e2e/**\"]\n",
        TWO
    ));
    assert_eq!(smoke(&named, &small).state, State::Ok);
}

#[test]
fn the_gate_wants_a_pass_in_30_days_and_the_scorecard_10_runs_with_reach() {
    let event = |at: &str, decision: &str, reach: bool| {
        let mut e = serde_json::json!({"at": at, "stage": "done", "decision": decision});
        if reach {
            e["reach"] = serde_json::json!({"all": false, "reached": 1, "suite": 2, "ran": 1});
        }
        e
    };
    let today = days("2026-03-01").unwrap();
    let fresh = [event("2026-02-20T10:00:00.000Z", "pass", true)];
    assert_eq!(
        gate(&fresh, today).text,
        "`fairlead done` last passed 9 days ago"
    );
    let stale = [event("2026-01-01T10:00:00.000Z", "pass", true)];
    assert_eq!(gate(&stale, today).state, State::Open);
    let failed = [event("2026-02-28T10:00:00.000Z", "fail", true)];
    let item = gate(&failed, today);
    assert_eq!(
        (item.state, item.fix.as_deref()),
        (State::Open, Some("fairlead done"))
    );
    let item = measure(&fresh);
    assert_eq!(item.state, State::Later);
    assert_eq!(item.text, "the scorecard needs 9 more gate runs");
    let ten: Vec<Value> = (0..10)
        .map(|_| event("2026-02-20T10:00:00.000Z", "pass", true))
        .collect();
    assert_eq!(measure(&ten).state, State::Ok);
    let bare: Vec<Value> = (0..10)
        .map(|_| event("2026-02-20T10:00:00.000Z", "pass", false))
        .collect();
    assert_eq!(measure(&bare).state, State::Later);
}

#[test]
fn the_binary_is_open_below_the_floor_and_the_fix_is_how_the_repository_installs_it() {
    let dir = std::env::temp_dir().join(format!("fl-setup-binary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let floor = config("fairlead = \"99.0\"\n");
    let item = binary(&dir, Some(&floor));
    assert_eq!(item.state, State::Open);
    assert!(
        item.text.starts_with("the config needs 99.0 or later"),
        "{}",
        item.text
    );
    assert_eq!(item.fix.as_deref(), Some(INSTALLER));
    std::fs::write(
        dir.join("package.json"),
        r#"{"devDependencies": {"fairlead": "^99.0.0"}}"#,
    )
    .unwrap();
    std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
    let item = binary(&dir, None);
    assert!(
        item.text.starts_with("package.json asks for ^99.0.0"),
        "{}",
        item.text
    );
    assert_eq!(item.fix.as_deref(), Some("pnpm install"));
    let _ = std::fs::remove_dir_all(&dir);
}
