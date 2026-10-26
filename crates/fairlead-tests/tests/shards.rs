//! Splitting a plan into shards: per-module runners keep their working
//! directories, a held test runs alone in the first shard only, and a
//! `{packages}` runner divides whole directories.

mod common;

use common::*;
use fairlead_core::plan::Plan;
use fairlead_tests::durations::Durations;
use fairlead_tests::shard::split;

const MODULES: &[(&str, &str)] = &[
    ("package.json", r#"{ "workspaces": ["tools/*"] }"#),
    ("tools/gen/package.json", r#"{ "name": "gen" }"#),
    ("tools/gen/test/g1.test.ts", "export {};\n"),
    ("tools/gen/test/g2.test.ts", "export {};\n"),
    ("tools/kit/package.json", r#"{ "name": "kit" }"#),
    ("tools/kit/test/k1.test.ts", "export {};\n"),
];

const JEST: &str = r#"
[[tests.runners]]
id = "jest"
match = ["tools/**"]
invoke = "per-module"
cwd = "{module}"
command = ["jest", "{files}"]
"#;

const HELD: &str = r#"
[[quarantine]]
path = "tools/gen/test/g2.test.ts"
signature = "ENOENT"
reason = "spawns a process from a URL path"
proved_in = "CI on Linux"
until = "2099-12-31"
"#;

fn changed_all(dir: &std::path::Path, toml: &str) -> (Plan, fairlead_core::config::Config) {
    let config = config(toml);
    let plan = run(
        dir,
        &config,
        vec![
            modified("tools/gen/test/g1.test.ts"),
            modified("tools/gen/test/g2.test.ts"),
            modified("tools/kit/test/k1.test.ts"),
        ],
    );
    (plan, config)
}

/// Each shard's invocations as (cwd, argv).
fn lists(plan: &Plan) -> Vec<Vec<(String, Vec<String>)>> {
    plan.shards
        .as_ref()
        .unwrap()
        .lists
        .iter()
        .map(|s| {
            s.invocations
                .iter()
                .map(|i| (i.cwd.clone(), i.argv.clone()))
                .collect()
        })
        .collect()
}

fn inv(cwd: &str, argv: &[&str]) -> (String, Vec<String>) {
    (cwd.into(), argv.iter().map(|a| a.to_string()).collect())
}

#[test]
fn a_per_module_runner_keeps_each_modules_directory_and_reads_durations_from_it() {
    let dir = repo("shard-modules", MODULES);
    let (mut plan, config) = changed_all(&dir, JEST);
    let mut durations = Durations::default();
    durations
        .add(
            &dir,
            "d.json",
            r#"{"test/g1.test.ts": 5, "tools/gen/test/g2.test.ts": 1, "test/k1.test.ts": 4}"#,
        )
        .unwrap();
    let scan = fairlead_lang::build(&dir, &config).unwrap();
    let done = split(&plan, &scan, &config, 2, Some(&durations)).unwrap();
    assert_eq!((done.timed, done.files), (3, 3));
    plan.shards = Some(done.shards);
    assert_eq!(
        lists(&plan),
        [
            vec![inv("tools/gen", &["jest", "test/g1.test.ts"])],
            vec![
                inv("tools/gen", &["jest", "test/g2.test.ts"]),
                inv("tools/kit", &["jest", "test/k1.test.ts"])
            ],
        ]
    );
}

#[test]
fn a_held_test_runs_alone_in_the_first_shard_and_is_never_divided() {
    let dir = repo("shard-held", MODULES);
    let (mut plan, config) = changed_all(&dir, &(JEST.to_string() + HELD));
    let scan = fairlead_lang::build(&dir, &config).unwrap();
    plan.shards = Some(split(&plan, &scan, &config, 2, None).unwrap().shards);
    let shards = plan.shards.as_ref().unwrap();
    assert!(shards
        .lists
        .iter()
        .all(|s| !s.files.contains(&"tools/gen/test/g2.test.ts".to_string())));
    let alone = |k: usize| {
        shards.lists[k]
            .invocations
            .iter()
            .filter(|i| i.quarantined.is_some())
            .count()
    };
    assert_eq!((alone(0), alone(1)), (1, 0));
}

const GO: &[(&str, &str)] = &[
    ("go.mod", "module example.com/shop\n\ngo 1.24\n"),
    ("pkg/price/price_test.go", "package price\n"),
    ("pkg/price/more_test.go", "package price\n"),
    ("cmd/shop/main_test.go", "package main\n"),
];

const GO_RUNNER: &str = r#"
[tests]
match = ["**/*_test.go"]

[[tests.runners]]
id = "go"
match = ["**/*_test.go"]
command = ["go", "test", "{packages}"]
"#;

#[test]
fn a_packages_runner_divides_whole_directories() {
    let dir = repo("shard-go", GO);
    let config = config(GO_RUNNER);
    let mut plan = run(
        &dir,
        &config,
        GO.iter().skip(1).map(|(p, _)| modified(p)).collect(),
    );
    let scan = fairlead_lang::build(&dir, &config).unwrap();
    plan.shards = Some(split(&plan, &scan, &config, 2, None).unwrap().shards);
    assert_eq!(
        lists(&plan),
        [
            vec![inv(".", &["go", "test", "./pkg/price"])],
            vec![inv(".", &["go", "test", "./cmd/shop"])],
        ]
    );
    let shards = plan.shards.unwrap();
    assert_eq!(shards.lists[0].weight, 2.0);
    assert_eq!(shards.lists[0].files.len(), 2);
}

#[test]
fn zero_shards_is_an_error() {
    let dir = repo("shard-zero", GO);
    let config = config(GO_RUNNER);
    let plan = run(&dir, &config, vec![modified("cmd/shop/main_test.go")]);
    let scan = fairlead_lang::build(&dir, &config).unwrap();
    assert!(split(&plan, &scan, &config, 0, None).is_err());
}
