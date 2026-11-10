//! The budget in `fairlead done`, through the binary: the nearest tests
//! run until the budget is spent and the rest is owed, a runner with no
//! recorded time runs a first few, a blanket runs the smoke set, and
//! `--full` or a budget of 0 runs the whole plan.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
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
    assert!(out.status.success(), "git {args:?}");
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

const RUNNER: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]
"#;

/// `src/a.ts`, `src/mid.ts` importing it, one test importing `a` and
/// `far` tests importing `mid`. `head` writes the change on a branch.
fn repo(name: &str, config: &str, far: usize, head: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-budget-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["src", "test"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    let mut files = vec![
        ("fairlead.toml".to_string(), config.to_string()),
        (
            ".gitignore".to_string(),
            "*.json\n!package.json\n".to_string(),
        ),
        (
            "package.json".to_string(),
            "{\"name\":\"app\",\"version\":\"1.0.0\"}\n".to_string(),
        ),
        ("flags.txt".to_string(), "on\n".to_string()),
        ("src/a.ts".to_string(), "export const a = 1;\n".to_string()),
        (
            "src/mid.ts".to_string(),
            "import { a } from './a';\nexport const mid = a;\n".to_string(),
        ),
        (
            "test/a.test.ts".to_string(),
            "import { a } from '../src/a';\n".to_string(),
        ),
    ];
    for i in 0..far {
        files.push((
            format!("test/m{i:02}.test.ts"),
            "import { mid } from '../src/mid';\n".to_string(),
        ));
    }
    for (path, text) in &files {
        std::fs::write(dir.join(path), text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    for (path, text) in head {
        std::fs::create_dir_all(dir.join(path).parent().unwrap()).unwrap();
        std::fs::write(dir.join(path), text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "change"]);
    dir
}

const EDIT: (&str, &str) = ("src/a.ts", "export const a = 2;\n");

/// A recorded run in which `unit` took `seconds` for `files` files.
fn timed(dir: &Path, seconds: f64, files: usize) {
    let log = dir.join(".git/fairlead");
    std::fs::create_dir_all(&log).unwrap();
    let line = serde_json::json!({
        "at": "2026-01-01T00:00:00Z", "stage": "done", "decision": "pass", "rules": [],
        "added": 0, "ms": 1.0, "fairlead": "0.19.0", "tree": "0",
        "steps": [{"id": "unit", "passed": true, "seconds": seconds, "files": files}],
    });
    std::fs::write(log.join("events.jsonl"), format!("{line}\n")).unwrap();
}

fn last_done(dir: &Path) -> Value {
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    log.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|e| e["stage"] == "done")
        .unwrap()
}

fn unit_step(out: &str) -> String {
    out.lines()
        .find(|l| l.contains("sh -c exit 0 sh"))
        .unwrap_or("")
        .to_string()
}

#[test]
fn the_nearest_tests_run_until_the_budget_is_spent_and_the_rest_is_owed() {
    let dir = repo("spent", RUNNER, 3, &[EDIT]);
    timed(&dir, 100.0, 10);
    let (code, out) = fairlead(&dir, &["done", "--budget", "25"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("unit: 2 of 4 reached test files run here; 2 owed to CI before merge: 2 past the 25 s budget"),
        "{out}"
    );
    let step = unit_step(&out);
    assert!(step.contains("test/a.test.ts test/m00.test.ts"), "{out}");
    assert!(!step.contains("m01"), "{out}");
    assert!(
        out.contains("done: passed, 2 steps; owed to CI before merge: unit 2 test files"),
        "{out}"
    );
    let event = last_done(&dir);
    assert_eq!(event["decision"], "pass");
    assert_eq!(event["budget_seconds"], 25);
    assert_eq!(event["debts"][0]["step"], "unit");
    assert_eq!(event["debts"][0]["files"], 2);
    assert_eq!(event["debts"][0]["why"], "budget");
    assert_eq!(event["debts"][0]["due"], "merge");
    assert_eq!(event["reach"]["reached"], 4);
    assert_eq!(event["reach"]["ran"], 2);
    assert_eq!(event["steps"][0]["files"], 2);
    let (code, out) = fairlead(&dir, &["done", "--check"]);
    assert_eq!(code, 0, "{out}");
    // The receipt says what is owed, and doesn't call the runner never run.
    let (_, out) = fairlead(&dir, &["receipt", "--base", "main"]);
    assert!(
        out.contains("owed     unit: 2 test files to CI before merge (2 budget)"),
        "{out}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_runner_with_no_recorded_time_runs_its_nearest_twenty() {
    let dir = repo("untimed", RUNNER, 24, &[EDIT]);
    let (code, out) = fairlead(&dir, &["done"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("unit: 20 of 25 reached test files run here; 5 owed to CI before merge: 5 past its first 20"),
        "{out}"
    );
    let step = unit_step(&out);
    assert!(step.contains("test/a.test.ts"), "{out}");
    assert!(step.contains("m18") && !step.contains("m19"), "{out}");
    assert_eq!(last_done(&dir)["debts"][0]["why"], "untimed");
    // That run recorded its time, so the next fits everything in the budget.
    std::fs::write(dir.join("src/a.ts"), "export const a = 3;\n").unwrap();
    let (code, out) = fairlead(&dir, &["done"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("owed to CI"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_changed_test_runs_whatever_the_budget() {
    let head = [(
        "test/m00.test.ts",
        "import { mid } from '../src/mid';\n// x\n",
    )];
    let dir = repo("changed", RUNNER, 2, &head);
    timed(&dir, 100.0, 10);
    let (code, out) = fairlead(&dir, &["done", "--budget", "1"]);
    assert_eq!(code, 0, "{out}");
    assert!(unit_step(&out).contains("test/m00.test.ts"), "{out}");
    assert!(!out.contains("owed to CI"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_blanket_runs_the_smoke_set_and_owes_the_rest() {
    let config =
        format!("{RUNNER}smoke = [\"test/a.test.ts\"]\n\n[plan]\nrun_all = [\"flags.txt\"]\n");
    let dir = repo("smoke", &config, 3, &[("flags.txt", "off\n")]);
    timed(&dir, 100.0, 10);
    let (code, out) = fairlead(&dir, &["done", "--budget", "25"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("unit: 1 of 4 reached test files run here; 3 owed to CI before merge: 3 reached only through a blanket"),
        "{out}"
    );
    let step = unit_step(&out);
    assert!(
        step.contains("test/a.test.ts") && !step.contains("m00"),
        "{out}"
    );
    assert!(!out.contains("advice"), "{out}");
    let event = last_done(&dir);
    assert_eq!(event["reach"]["all"], true);
    assert_eq!(event["debts"][0]["why"], "blanket");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_blanket_on_an_untimed_runner_with_no_smoke_set_runs_a_trial_that_times_it() {
    let config = format!("{RUNNER}\n[plan]\nrun_all = [\"flags.txt\"]\n");
    let dir = repo("blanketuntimed", &config, 3, &[("flags.txt", "off\n")]);
    let (code, out) = fairlead(&dir, &["done"]);
    assert_eq!(code, 0, "{out}");
    // All four are reached only through the blanket, and no time is on record.
    let step = unit_step(&out);
    assert!(
        step.contains("test/a.test.ts") && step.contains("m02"),
        "{out}"
    );
    assert!(!out.contains("owed to CI"), "{out}");
    assert_eq!(last_done(&dir)["reach"]["ran"], 4);

    // With a smoke set the untimed runner runs that and owes the rest.
    let smoked =
        format!("{RUNNER}smoke = [\"test/a.test.ts\"]\n\n[plan]\nrun_all = [\"flags.txt\"]\n");
    let dir2 = repo("blanketuntimedsmoke", &smoked, 3, &[("flags.txt", "off\n")]);
    let (code, out) = fairlead(&dir2, &["done"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("unit: 1 of 4 reached test files run here; 3 owed to CI before merge"),
        "{out}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&dir2).unwrap();
}

#[test]
fn a_blanket_with_no_smoke_set_says_so_and_runs_whole_when_it_fits() {
    let config = format!("{RUNNER}\n[plan]\nrun_all = [\"flags.txt\"]\n");
    let dir = repo("nosmoke", &config, 3, &[("flags.txt", "off\n")]);
    timed(&dir, 100.0, 10);
    let (code, out) = fairlead(&dir, &["done", "--budget", "25"]);
    assert_eq!(code, 0, "{out}");
    assert!(unit_step(&out).is_empty(), "{out}");
    assert!(
        out.contains("advice: unit: 4 test files are reached only through a blanket and the runner names no smoke set"),
        "{out}"
    );
    assert!(
        out.contains("advice is for the pull request's notes"),
        "{out}"
    );
    // Four files at ten seconds each fit a budget of sixty: the suite runs whole.
    let (code, out) = fairlead(&dir, &["done", "--budget", "60"]);
    assert_eq!(code, 0, "{out}");
    assert!(unit_step(&out).contains("m02"), "{out}");
    assert!(!out.contains("owed to CI"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn full_and_a_budget_of_zero_run_the_whole_plan() {
    let dir = repo("full", RUNNER, 3, &[EDIT]);
    timed(&dir, 100.0, 10);
    for args in [
        &["done", "--full"][..],
        &["done", "--budget", "0"][..],
        &["done", "--set", "done.budget_seconds=0"][..],
    ] {
        let (code, out) = fairlead(&dir, args);
        assert_eq!(code, 0, "{out}");
        assert!(!out.contains("owed to CI"), "{args:?}: {out}");
        assert!(last_done(&dir).get("debts").is_none(), "{args:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_early_project_owes_its_debts_to_the_nightly_run() {
    let config = format!("phase = \"early\"\n{RUNNER}");
    let dir = repo("early", &config, 3, &[EDIT]);
    timed(&dir, 100.0, 10);
    let (code, out) = fairlead(&dir, &["done", "--budget", "25"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("2 owed to CI by the nightly run"), "{out}");
    assert_eq!(last_done(&dir)["debts"][0]["due"], "nightly");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_new_file_no_test_is_near_gets_advice() {
    let head = [("src/lonely.ts", "export const lonely = 1;\n")];
    let dir = repo("lonely", RUNNER, 1, &head);
    let (code, out) = fairlead(&dir, &["done"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("advice: src/lonely.ts is new and no test is near it"),
        "{out}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_new_file_named_like_a_test_is_not_asked_for_a_test() {
    // Outside `tests.match`, so the planner reads it as a source file.
    let config = format!("[tests]\nmatch = {{ replace = [\"test/**\"] }}\n{RUNNER}");
    let head = [("scripts/check.test.ts", "export {};\n")];
    let dir = repo("specnew", &config, 1, &head);
    let (code, out) = fairlead(&dir, &["done"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("no test is near"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_new_file_with_a_test_beside_it_gets_none() {
    let head = [
        ("src/kept.ts", "export const kept = 1;\n"),
        ("test/kept.test.ts", "import { kept } from '../src/kept';\n"),
    ];
    let dir = repo("kept", RUNNER, 1, &head);
    let (code, out) = fairlead(&dir, &["done"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("advice"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}
