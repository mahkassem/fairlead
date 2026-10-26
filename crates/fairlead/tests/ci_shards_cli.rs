//! `fairlead ci plan --shards N` and `ci run --shard K/N` on a real
//! repository: balancing by durations and by file count, every invocation
//! once across the matrix, a mismatched shard refused, and the results of
//! N jobs reported as one run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    // A clean environment, so a developer's FAIRLEAD_*, CI, git or GitHub settings can't leak in.
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let out = command(dir, "git", args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

// Each invocation appends a line to `ran.log`: its id, then any files.
const CONFIG: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "echo unit \"$@\" >> ran.log", "sh", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["sh", "-c", "echo e2e \"$@\" >> ran.log", "sh", "{files}"]
all_command = ["sh", "-c", "echo e2e-all >> ran.log"]

[[tests.runners]]
id = "whole"
match = ["spec/**"]
command = ["sh", "-c", "echo whole >> ran.log"]

[[checks]]
id = "lint"
command = ["sh", "-c", "echo lint >> ran.log"]
paths = ["src/**"]
"#;

const UNIT: [&str; 4] = [
    "test/a.test.ts",
    "test/b.test.ts",
    "test/c.test.ts",
    "test/d.test.ts",
];

/// A repository whose change to `src/x.ts` reaches four unit tests, one
/// end-to-end spec and one whole-runner spec.
fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-shards-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["src", "test", "e2e", "spec"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join(".gitignore"), "ran.log\n*.json\n*.xml\n").unwrap();
    std::fs::write(dir.join("src/x.ts"), "export const x = 1;\n").unwrap();
    for file in UNIT.iter().chain(&["e2e/flow.spec.ts", "spec/all.spec.ts"]) {
        std::fs::write(dir.join(file), "import { x } from '../src/x';\n").unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("src/x.ts"), "export const x = 2;\n").unwrap();
    dir
}

fn plan(dir: &Path, out: &str, extra: &[&str]) -> Value {
    let mut args = vec!["ci", "plan", "--base", "main", "--out", out];
    args.extend(extra);
    let (code, text) = fairlead(dir, &args);
    assert_eq!(code, 0, "{text}");
    serde_json::from_str(&std::fs::read_to_string(dir.join(out)).unwrap()).unwrap()
}

/// Each shard's divided files.
fn files(plan: &Value) -> Vec<Vec<String>> {
    plan["shards"]["lists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            s["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap().to_string())
                .collect()
        })
        .collect()
}

fn weights(plan: &Value) -> Vec<f64> {
    plan["shards"]["lists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["weight"].as_f64().unwrap())
        .collect()
}

#[test]
fn a_plan_without_shards_is_the_plan_it_was_and_shards_add_only_their_key() {
    let dir = repo("unchanged");
    let plain = plan(&dir, "plain.json", &[]);
    let text = std::fs::read_to_string(dir.join("plain.json")).unwrap();
    assert!(!text.contains("shards"), "{text}");
    let mut sharded = plan(&dir, "sharded.json", &["--shards", "2"]);
    assert!(sharded.as_object_mut().unwrap().remove("shards").is_some());
    assert_eq!(plain, sharded);
}

#[test]
fn without_durations_the_shards_balance_by_file_count() {
    let dir = repo("count");
    let p = plan(&dir, "plan.json", &["--shards", "2"]);
    assert_eq!(p["shards"]["by"], "files");
    assert_eq!(p["shards"]["split"], serde_json::json!(["unit", "e2e"]));
    let lists = files(&p);
    assert_eq!(
        lists[0],
        ["test/a.test.ts", "test/c.test.ts", "e2e/flow.spec.ts"]
    );
    assert_eq!(lists[1], ["test/b.test.ts", "test/d.test.ts"]);
    assert_eq!(weights(&p), [3.0, 2.0]);
}

const SECONDS: &str = r#"{"test/a.test.ts": 10, "test/b.test.ts": 1, "test/c.test.ts": 1, "test/d.test.ts": 8, "e2e/flow.spec.ts": 3}"#;

#[test]
fn json_durations_balance_the_longest_first() {
    let dir = repo("json");
    std::fs::write(dir.join("durations.json"), SECONDS).unwrap();
    let (code, out) = fairlead(
        &dir,
        &[
            "ci",
            "plan",
            "--base",
            "main",
            "--out",
            "plan.json",
            "--shards",
            "2",
            "--durations",
            "durations.json",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("by durations, 5 of 5 files timed"), "{out}");
    let p: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("plan.json")).unwrap()).unwrap();
    assert_eq!(p["shards"]["by"], "durations");
    // 10, then 8 to the empty one, 3 to the lighter, then 1 and 1, the tie to the first.
    assert_eq!(
        files(&p),
        [
            vec!["test/a.test.ts", "test/b.test.ts", "test/c.test.ts"],
            vec!["test/d.test.ts", "e2e/flow.spec.ts"]
        ]
    );
    assert_eq!(weights(&p), [12.0, 11.0]);
}

#[test]
fn a_junit_report_gives_the_same_shards_as_the_same_seconds_in_json() {
    let dir = repo("junit");
    std::fs::write(dir.join("durations.json"), SECONDS).unwrap();
    let report = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites>
  <testsuite name="a" file="test/a.test.ts"><testcase name="1" time="4"/><testcase name="2" time="6"/></testsuite>
  <testsuite name="rest">
    <testcase name="b" file="test/b.test.ts" time="1"/>
    <testcase name="c" classname="test/c.test.ts" time="1"/>
    <testcase name="d" file="test/d.test.ts" time="8"/>
  </testsuite>
  <testsuite name="flow" file="e2e/flow.spec.ts" time="3"></testsuite>
</testsuites>
"#;
    std::fs::write(dir.join("report.xml"), report).unwrap();
    let json = plan(
        &dir,
        "json.json",
        &["--shards", "2", "--durations", "durations.json"],
    );
    let xml = plan(
        &dir,
        "xml.json",
        &["--shards", "2", "--durations", "report.xml"],
    );
    assert_eq!(json["shards"], xml["shards"]);
}

#[test]
fn a_file_with_no_duration_weighs_the_median_of_the_known_ones() {
    let dir = repo("median");
    std::fs::write(
        dir.join("durations.json"),
        r#"{"test/a.test.ts": 10, "test/b.test.ts": 2, "test/c.test.ts": 4, "e2e/flow.spec.ts": 20}"#,
    )
    .unwrap();
    let p = plan(
        &dir,
        "plan.json",
        &["--shards", "3", "--durations", "durations.json"],
    );
    // d has none, so it weighs 7, the median of 2, 4, 10 and 20; at 1 it
    // would have joined c and b in the third shard.
    assert_eq!(files(&p)[1], ["test/a.test.ts", "test/b.test.ts"]);
    assert_eq!(files(&p)[2], ["test/c.test.ts", "test/d.test.ts"]);
    assert_eq!(weights(&p), [20.0, 12.0, 11.0]);
}

#[test]
fn when_no_file_has_a_duration_each_weighs_one() {
    let dir = repo("unknown");
    std::fs::write(dir.join("durations.json"), r#"{"elsewhere.test.ts": 50}"#).unwrap();
    let (code, out) = fairlead(
        &dir,
        &[
            "ci",
            "plan",
            "--base",
            "main",
            "--out",
            "plan.json",
            "--shards",
            "3",
            "--durations",
            "durations.json",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("by durations, 0 of 5 files timed"), "{out}");
    let p: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("plan.json")).unwrap()).unwrap();
    assert_eq!(weights(&p), [2.0, 2.0, 1.0]);
}

#[test]
fn the_same_plan_and_durations_give_the_same_shards() {
    let dir = repo("same");
    std::fs::write(dir.join("durations.json"), SECONDS).unwrap();
    let args = ["--shards", "3", "--durations", "durations.json"];
    let first = plan(&dir, "one.json", &args);
    let second = plan(&dir, "two.json", &args);
    assert_eq!(first["shards"], second["shards"]);
    assert_eq!(
        std::fs::read_to_string(dir.join("one.json")).unwrap(),
        std::fs::read_to_string(dir.join("two.json")).unwrap()
    );
}

#[test]
fn a_shard_the_plan_doesnt_have_is_refused() {
    let dir = repo("mismatch");
    plan(&dir, "plain.json", &[]);
    plan(&dir, "plan.json", &["--shards", "2"]);
    for (file, shard, says) in [
        ("plan.json", "1/3", "the plan has 2 shards, not 3"),
        ("plan.json", "3/2", "K runs from 1 to 2"),
        ("plan.json", "two", "write it as K/N"),
        ("plain.json", "1/2", "the plan has no shards"),
    ] {
        let (code, out) = fairlead(&dir, &["ci", "run", "--plan", file, "--shard", shard]);
        assert_eq!(code, 2, "{shard}: {out}");
        assert!(out.contains(says), "{shard}: {out}");
    }
    assert!(!dir.join("ran.log").exists(), "nothing ran");
    let (code, out) = fairlead(
        &dir,
        &["ci", "plan", "--base", "main", "--durations", "d.json"],
    );
    assert_eq!(code, 2, "durations without shards: {out}");
}

/// Runs every shard of a fresh plan and counts each logged line.
fn run_matrix(dir: &Path, n: u32, extra: &[&str]) -> (Vec<String>, BTreeMap<String, usize>) {
    let count = n.to_string();
    let mut args = vec!["--shards", count.as_str()];
    args.extend(extra);
    plan(dir, "plan.json", &args);
    let mut first = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for k in 1..=n {
        let _ = std::fs::remove_file(dir.join("ran.log"));
        let shard = format!("{k}/{n}");
        let (code, out) = fairlead(
            dir,
            &["ci", "run", "--plan", "plan.json", "--shard", &shard],
        );
        assert_eq!(code, 0, "{shard}: {out}");
        let log = std::fs::read_to_string(dir.join("ran.log")).unwrap_or_default();
        for line in log.lines() {
            let mut words = line.split_whitespace();
            let id = words.next().unwrap().to_string();
            if k == 1 {
                first.push(id.clone());
            }
            let files: Vec<&str> = words.collect();
            if files.is_empty() {
                *seen.entry(id).or_default() += 1;
            }
            for f in files {
                *seen.entry(f.to_string()).or_default() += 1;
            }
        }
    }
    (first, seen)
}

#[test]
fn across_the_matrix_every_invocation_and_file_runs_exactly_once() {
    let dir = repo("once");
    for n in 1..=7 {
        let (first, seen) = run_matrix(&dir, n, &[]);
        let want: BTreeMap<String, usize> = UNIT
            .iter()
            .chain(&["e2e/flow.spec.ts", "whole", "lint"])
            .map(|s| (s.to_string(), 1))
            .collect();
        assert_eq!(seen, want, "with {n} shards");
        assert!(
            first.contains(&"whole".to_string()) && first.contains(&"lint".to_string()),
            "{first:?}"
        );
    }
}

#[test]
fn a_plan_that_runs_everything_splits_runners_by_their_own_files_and_runs_an_all_command_once() {
    let dir = repo("all");
    for n in 1..=3 {
        let (first, seen) = run_matrix(&dir, n, &["--set", "plan.run_all=[\"src/**\"]"]);
        let want: BTreeMap<String, usize> = UNIT
            .iter()
            .chain(&["e2e-all", "whole", "lint"])
            .map(|s| (s.to_string(), 1))
            .collect();
        assert_eq!(seen, want, "with {n} shards");
        assert!(first.contains(&"e2e-all".to_string()), "{first:?}");
    }
    let p: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("plan.json")).unwrap()).unwrap();
    assert_eq!(p["all"], true);
    assert_eq!(p["shards"]["split"], serde_json::json!(["unit"]));
}

#[test]
fn a_sharded_plan_run_without_a_shard_still_runs_everything_once() {
    let dir = repo("unsharded-run");
    plan(&dir, "plan.json", &["--shards", "3"]);
    let (code, out) = fairlead(&dir, &["ci", "run", "--plan", "plan.json"]);
    assert_eq!(code, 0, "{out}");
    let log = std::fs::read_to_string(dir.join("ran.log")).unwrap();
    assert_eq!(log.lines().count(), 4, "{log}");
    assert!(
        log.contains("unit test/a.test.ts test/b.test.ts test/c.test.ts test/d.test.ts"),
        "{log}"
    );
}

#[test]
fn the_report_merges_each_shards_results_and_says_when_one_is_missing() {
    let dir = repo("report");
    plan(&dir, "plan.json", &["--shards", "2"]);
    for k in 1..=2 {
        let shard = format!("{k}/2");
        let results = format!("r{k}.json");
        let (code, out) = fairlead(
            &dir,
            &[
                "ci",
                "run",
                "--plan",
                "plan.json",
                "--shard",
                &shard,
                "--results",
                &results,
            ],
        );
        assert_eq!(code, 0, "{out}");
        let r: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(&results)).unwrap()).unwrap();
        assert_eq!(r["shard"], shard.as_str());
    }
    let (code, out) = fairlead(
        &dir,
        &[
            "ci",
            "report",
            "--plan",
            "plan.json",
            "--results",
            "r2.json",
            "--results",
            "r1.json",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("### Fairlead: passed"), "{out}");
    assert!(
        out.contains("Split across 2 shards by file count."),
        "{out}"
    );
    assert_eq!(out.matches("| `unit` in `.` | passed").count(), 2, "{out}");
    assert!(out.contains("| `lint` in `.` | passed"), "{out}");
    assert!(!out.contains("Incomplete"), "{out}");

    let (code, out) = fairlead(
        &dir,
        &[
            "ci",
            "report",
            "--plan",
            "plan.json",
            "--results",
            "r1.json",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("### Fairlead: incomplete"), "{out}");
    assert!(out.contains("no results from shard 2,"), "{out}");

    let (code, out) = fairlead(
        &dir,
        &[
            "ci",
            "report",
            "--plan",
            "plan.json",
            "--results",
            "r1.json",
            "--results",
            "r1.json",
        ],
    );
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("two results files are for shard 1/2"), "{out}");
}
