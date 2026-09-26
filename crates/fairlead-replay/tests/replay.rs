//! A replay over a synthetic git history with known failures reports
//! exactly the expected hits, misses and other outcomes.

use std::path::{Path, PathBuf};
use std::process::Command;

use fairlead_core::config::Config;
use fairlead_replay::dataset::{Job, Row};
use fairlead_replay::git::Worktree;
use fairlead_replay::report::report;
use fairlead_replay::run::{replay, Replayer, Sources};
use fairlead_replay::window::Window;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

struct History {
    dir: PathBuf,
    base: String,
    a: String,
    b: String,
    c: String,
    d: String,
}

fn history() -> History {
    let dir = std::env::temp_dir().join(format!("fairlead-replay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(&dir, "src/b.ts", "export const b = 1;\n");
    write(&dir, "test/a.test.ts", "import { a } from '../src/a';\n");
    write(&dir, "test/b.test.ts", "import { b } from '../src/b';\n");
    let base = commit(&dir, "base");
    git(&dir, &["checkout", "-q", "-b", "pr7"]);
    write(&dir, "src/a.ts", "export const a = 2;\n");
    let a = commit(&dir, "pr7: change a");
    write(&dir, "README.md", "# notes\n");
    let b = commit(&dir, "pr7: notes only");
    git(&dir, &["checkout", "-q", "-b", "pr8", &base]);
    write(&dir, "src/a.ts", "export const a = 3;\n");
    let c = commit(&dir, "pr8: change a");
    git(&dir, &["checkout", "-q", "-b", "pr10", &base]);
    write(&dir, "e2e/d.test.ts", "import { b } from '../src/b';\n");
    let d = commit(&dir, "pr10: a test two runners match");
    git(&dir, &["checkout", "-q", "main"]);
    History {
        dir,
        base,
        a,
        b,
        c,
        d,
    }
}

fn job(name: &str, conclusion: &str, log: &[&str]) -> Job {
    Job {
        name: name.into(),
        conclusion: conclusion.into(),
        failed_steps: Vec::new(),
        annotations: Vec::new(),
        annotations_capped: false,
        log: log.iter().map(|s| s.to_string()).collect(),
    }
}

fn row(
    run_id: u64,
    attempt: u32,
    pr: u64,
    head: &str,
    base: &str,
    day: u32,
    jobs: Vec<Job>,
) -> Row {
    let conclusion = if jobs.iter().any(Job::failed) {
        "failure"
    } else {
        "success"
    };
    Row {
        repo: "example/repo".into(),
        run_id,
        attempt,
        event: "pull_request".into(),
        workflow: "ci".into(),
        pr: Some(pr),
        head_sha: head.into(),
        base_sha: Some(base.into()),
        created_at: format!("2026-09-{day:02}T10:00:00Z"),
        conclusion: conclusion.into(),
        jobs,
    }
}

const CONFIG: &str = r#"
[[tests.runners]]
id = "vitest"
match = ["**"]
command = ["vitest", "run", "{files}"]

[[tests.runners]]
id = "e2e"
match = ["e2e/**"]
command = ["playwright", "test", "{files}"]

[[replay.failures]]
runner = "vitest"
extractor = "vitest"
job = "^test$"

[replay]
ignore = ["^all-green$"]
ignore_steps = ["^Install$"]
"#;

#[test]
fn a_synthetic_history_replays_to_the_expected_outcomes() {
    let h = history();
    let fail_a = " FAIL  test/a.test.ts > a works";
    let fail_b = " FAIL  test/b.test.ts > b works";
    let rows = vec![
        row(
            1,
            1,
            7,
            &h.a,
            &h.base,
            10,
            vec![job("test", "failure", &[fail_a, fail_b])],
        ),
        row(
            2,
            1,
            7,
            &h.a,
            &h.base,
            11,
            vec![job("test", "failure", &[fail_b])],
        ),
        row(
            2,
            2,
            7,
            &h.a,
            &h.base,
            11,
            vec![job("test", "success", &[])],
        ),
        row(
            4,
            1,
            7,
            &h.b,
            &h.base,
            12,
            vec![job("test", "success", &[])],
        ),
        row(
            5,
            1,
            8,
            &h.c,
            &h.base,
            13,
            vec![
                job("test", "failure", &[fail_b]),
                job("docs", "failure", &[]),
                job("all-green", "failure", &[]),
                Job {
                    failed_steps: vec!["Install".into()],
                    ..job("test-windows", "failure", &[])
                },
            ],
        ),
        row(
            6,
            1,
            9,
            "0123456789abcdef0123456789abcdef01234567",
            &h.base,
            14,
            vec![job("test", "failure", &[fail_b])],
        ),
        row(
            7,
            1,
            8,
            &h.c,
            &h.base,
            15,
            vec![job("test", "failure", &["Error: out of memory"])],
        ),
        row(
            8,
            1,
            8,
            &h.c,
            &h.base,
            16,
            vec![job("lint", "failure", &[" FAIL  test/b.test.ts > b"])],
        ),
        row(
            10,
            1,
            10,
            &h.d,
            &h.base,
            14,
            vec![job("test", "failure", &[fail_b])],
        ),
        row(
            9,
            1,
            8,
            &h.c,
            &h.base,
            1,
            vec![job("test", "failure", &[fail_b])],
        ),
    ];
    let mut config: Config = toml::from_str(CONFIG).unwrap();
    config.graph.cache = false;
    let wt_path = std::env::temp_dir().join(format!("fairlead-replay-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt_path);
    let replayer = Replayer {
        clone: &h.dir,
        worktree: Worktree::open(&h.dir, &wt_path, &h.base).unwrap(),
        config: &config,
        sources: Sources::new(&config).unwrap(),
    };
    let window = Window::ending("2026-09-16", 7).unwrap();
    let replayed = replay(&replayer, &rows, &window);
    let r = report("example/repo", &window, 30, &replayed);
    assert_eq!(r.runs, 7, "run 9 is outside the window and run 4 passed");
    assert_eq!(r.hits, 1, "test/a.test.ts in run 1");
    assert_eq!(r.misses.len(), 1, "{:?}", r.misses);
    assert_eq!(r.misses[0].run_id, 5);
    assert_eq!(r.misses[0].target, "test/b.test.ts");
    assert_eq!(
        r.misses[0].fix,
        "[[tests.owners]] match = \"test/**\", covers = [\"src/**\"]"
    );
    assert_eq!(r.flaky, 1, "run 2 passed on its second attempt");
    assert_eq!(
        r.unconfirmed, 1,
        "run 1's b passed when the same head ran again in run 2"
    );
    assert_eq!(r.unavailable, 1, "run 6's commit isn't in the clone");
    assert_eq!(r.unattributed, 1, "run 7 names no file");
    assert_eq!(r.errors, 1, "run 10's commit has a test two runners match");
    assert_eq!(r.recall, Some(0.5));
    assert_eq!(r.hits_selected, 1);
    assert_eq!(
        r.strict_recall,
        Some(1.0 / 3.0),
        "unconfirmed counts as a miss"
    );
    assert_eq!(
        r.unwatched.get("docs"),
        Some(&1),
        "no rule names the docs job"
    );
    assert_eq!(r.unwatched.len(), 2, "{:?}", r.unwatched);
    assert_eq!(
        r.ignored, 2,
        "all-green by name, the install failure by step"
    );
    let pr = &r.by_event["pull_request"];
    assert_eq!((pr.hits, pr.misses, pr.unconfirmed), (1, 1, 1));
    assert!(r.first_plan_seconds.is_some() && r.p90_plan_seconds.is_some());
}

#[test]
fn every_job_is_attributed_at_the_failing_commit_even_after_judging_another() {
    let dir = std::env::temp_dir().join(format!("fairlead-replay-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "src/b.ts", "export const b = 1;\n");
    write(&dir, "test/a.test.ts", "it('a works', () => {});\n");
    write(&dir, "test/b.test.ts", "import { b } from '../src/b';\n");
    write(&dir, "one/x.test.ts", "it('special title', () => {});\n");
    write(&dir, "two/x.test.ts", "it('other title', () => {});\n");
    let base = commit(&dir, "base");
    git(&dir, &["checkout", "-q", "-b", "pr11"]);
    write(&dir, "src/b.ts", "export const b = 2;\n");
    let first = commit(&dir, "pr11: change b");
    write(&dir, "one/x.test.ts", "it('other title', () => {});\n");
    write(&dir, "two/x.test.ts", "it('special title', () => {});\n");
    let later = commit(&dir, "pr11: swap the titles");
    git(&dir, &["checkout", "-q", "main"]);
    let rows = vec![
        row(
            1,
            1,
            11,
            &first,
            &base,
            10,
            vec![
                job("test", "failure", &[" FAIL  test/a.test.ts > a works"]),
                job("test2", "failure", &[" FAIL  x.test.ts > special title"]),
            ],
        ),
        row(
            2,
            1,
            11,
            &later,
            &base,
            11,
            vec![job("test", "success", &[])],
        ),
    ];
    let config_text = r#"
[[tests.runners]]
id = "vitest"
match = ["**"]
command = ["vitest", "run", "{files}"]

[[replay.failures]]
runner = "vitest"
extractor = "vitest"
job = "^test2?$"
"#;
    let mut config: Config = toml::from_str(config_text).unwrap();
    config.graph.cache = false;
    let wt_path =
        std::env::temp_dir().join(format!("fairlead-replay-order-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt_path);
    let replayer = Replayer {
        clone: &dir,
        worktree: Worktree::open(&dir, &wt_path, &base).unwrap(),
        config: &config,
        sources: Sources::new(&config).unwrap(),
    };
    let window = Window::ending("2026-09-11", 7).unwrap();
    let replayed = replay(&replayer, &rows, &window);
    let named: Vec<String> = replayed
        .failures
        .iter()
        .filter(|f| f.job == "test2")
        .map(|f| format!("{:?}", f.target))
        .collect();
    assert_eq!(
        named,
        [r#"Test("one/x.test.ts")"#],
        "{:?}",
        replayed.failures
    );
    let a = replayed.failures.iter().find(|f| f.job == "test").unwrap();
    assert_eq!(
        format!("{:?}", a.outcome),
        "Miss",
        "the later head passed with a different change"
    );
}

#[test]
fn a_rebased_head_that_passed_leaves_the_failure_unconfirmed() {
    let dir = std::env::temp_dir().join(format!("fairlead-replay-rebase-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "src/b.ts", "export const b = 1;\n");
    write(&dir, "src/c.ts", "export const c = 1;\n");
    write(&dir, "test/a.test.ts", "it('a works', () => {});\n");
    write(&dir, "test/b.test.ts", "import { b } from '../src/b';\n");
    write(&dir, "test/c.test.ts", "import { c } from '../src/c';\n");
    let base = commit(&dir, "base");
    git(&dir, &["checkout", "-q", "-b", "pr12"]);
    write(&dir, "src/b.ts", "export const b = 2;\n");
    let first = commit(&dir, "pr12: change b");
    git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "src/c.ts", "export const c = 2;\n");
    let newer = commit(&dir, "main moves on");
    git(&dir, &["checkout", "-q", "-b", "pr12-rebased"]);
    git(&dir, &["cherry-pick", &first]);
    let rebased = git(&dir, &["rev-parse", "HEAD"]);
    git(&dir, &["checkout", "-q", "main"]);
    let fail = || vec![job("test", "failure", &[" FAIL  test/a.test.ts > a works"])];
    let rows = vec![
        row(1, 1, 12, &first, &base, 10, fail()),
        row(
            2,
            1,
            12,
            &rebased,
            &newer,
            11,
            vec![job("test", "success", &[])],
        ),
    ];
    let mut config: Config = toml::from_str(CONFIG).unwrap();
    config.graph.cache = false;
    let wt_path =
        std::env::temp_dir().join(format!("fairlead-replay-rebase-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt_path);
    let replayer = Replayer {
        clone: &dir,
        worktree: Worktree::open(&dir, &wt_path, &base).unwrap(),
        config: &config,
        sources: Sources::new(&config).unwrap(),
    };
    let replayed = replay(&replayer, &rows, &Window::ending("2026-09-11", 7).unwrap());
    let outcomes: Vec<String> = replayed
        .failures
        .iter()
        .map(|f| format!("{:?}", f.outcome))
        .collect();
    assert_eq!(outcomes, ["Unconfirmed"]);
}

#[test]
fn a_hit_in_a_plan_that_selects_everything_counts_as_run_all() {
    let dir = std::env::temp_dir().join(format!("fairlead-replay-all-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "package.json", "{ \"name\": \"root\" }\n");
    write(&dir, "test/a.test.ts", "it('a works', () => {});\n");
    let base = commit(&dir, "base");
    write(
        &dir,
        "package.json",
        "{ \"name\": \"root\", \"private\": true }\n",
    );
    let head = commit(&dir, "change the root manifest");
    let rows = vec![row(
        1,
        1,
        13,
        &head,
        &base,
        10,
        vec![job("test", "failure", &[" FAIL  test/a.test.ts > a works"])],
    )];
    let mut config: Config = toml::from_str(CONFIG).unwrap();
    config.graph.cache = false;
    let wt_path =
        std::env::temp_dir().join(format!("fairlead-replay-all-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt_path);
    let replayer = Replayer {
        clone: &dir,
        worktree: Worktree::open(&dir, &wt_path, &base).unwrap(),
        config: &config,
        sources: Sources::new(&config).unwrap(),
    };
    let replayed = replay(&replayer, &rows, &Window::ending("2026-09-11", 7).unwrap());
    let r = report(
        "example/repo",
        &Window::ending("2026-09-11", 7).unwrap(),
        30,
        &replayed,
    );
    assert_eq!((r.hits, r.hits_run_all, r.hits_selected), (1, 1, 0));
    assert_eq!(r.widened_by.get("run-all package.json"), Some(&1));
}

#[test]
fn a_commit_the_remote_lost_doesnt_keep_the_rest_of_its_batch_out() {
    let origin =
        std::env::temp_dir().join(format!("fairlead-replay-origin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&origin);
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "main"]);
    write(&origin, "a.txt", "a\n");
    commit(&origin, "a");
    let clone = std::env::temp_dir().join(format!("fairlead-replay-lost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&clone);
    git(
        std::env::temp_dir().as_path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    git(&origin, &["checkout", "-q", "-b", "later"]);
    write(&origin, "b.txt", "b\n");
    let later = commit(&origin, "b");
    git(
        &origin,
        &["config", "uploadpack.allowAnySHA1InWant", "true"],
    );
    let gone = "0123456789abcdef0123456789abcdef01234567".to_string();
    let missing = fairlead_replay::git::fetch_missing(&clone, &[gone, later.clone()]);
    assert_eq!(missing, 1, "only the lost commit is still missing");
    assert!(fairlead_replay::git::has_commit(&clone, &later));
}

#[test]
fn a_row_without_a_base_is_planned_from_the_default_branch() {
    let origin =
        std::env::temp_dir().join(format!("fairlead-replay-nobase-o-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&origin);
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "main"]);
    write(&origin, "src/b.ts", "export const b = 1;\n");
    write(&origin, "test/a.test.ts", "it('a works', () => {});\n");
    write(&origin, "test/b.test.ts", "import { b } from '../src/b';\n");
    commit(&origin, "base");
    git(&origin, &["checkout", "-q", "-b", "fork-pr"]);
    write(&origin, "src/b.ts", "export const b = 2;\n");
    let head = commit(&origin, "change b");
    git(&origin, &["checkout", "-q", "main"]);
    let clone =
        std::env::temp_dir().join(format!("fairlead-replay-nobase-c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&clone);
    git(
        std::env::temp_dir().as_path(),
        &[
            "clone",
            "-q",
            "--no-checkout",
            origin.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    git(&clone, &["fetch", "-q", "origin", "fork-pr"]);
    let mut failing = row(
        1,
        1,
        14,
        &head,
        "unused",
        10,
        vec![job("test", "failure", &[" FAIL  test/b.test.ts > b works"])],
    );
    failing.base_sha = None;
    failing.created_at = "2099-01-01T00:00:00Z".into();
    let mut config: Config = toml::from_str(CONFIG).unwrap();
    config.graph.cache = false;
    let wt_path =
        std::env::temp_dir().join(format!("fairlead-replay-nobase-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt_path);
    let replayer = Replayer {
        clone: &clone,
        worktree: Worktree::open(&clone, &wt_path, &head).unwrap(),
        config: &config,
        sources: Sources::new(&config).unwrap(),
    };
    let window = Window::ending("2099-01-01", 7).unwrap();
    let replayed = replay(&replayer, &[failing], &window);
    let outcomes: Vec<String> = replayed
        .failures
        .iter()
        .map(|f| format!("{:?}", f.outcome))
        .collect();
    assert_eq!(outcomes, ["Hit"], "{:?}", replayed.failures);
    assert_eq!(replayed.failures[0].changed, ["src/b.ts"]);
}

#[test]
fn a_quarantine_entry_applies_only_while_the_dataset_bears_it_out() {
    let dir = std::env::temp_dir().join(format!("fairlead-replay-q-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "src/b.ts", "export const b = 1;\n");
    write(&dir, "test/a.test.ts", "it('a works', () => {});\n");
    write(&dir, "test/b.test.ts", "import { b } from '../src/b';\n");
    let base = commit(&dir, "base");
    let mut heads = Vec::new();
    for n in 0..3 {
        git(&dir, &["checkout", "-q", "-B", &format!("pr{n}"), &base]);
        write(&dir, "src/b.ts", &format!("export const b = {};\n", n + 2));
        heads.push(commit(&dir, &format!("pr{n}")));
    }
    git(&dir, &["checkout", "-q", "-B", "pr3", &base]);
    write(
        &dir,
        "test/a.test.ts",
        "it('a works', () => { void 0; });\n",
    );
    let edits_a = commit(&dir, "pr3: edit a");
    git(&dir, &["checkout", "-q", "main"]);
    let fail_a = " FAIL  test/a.test.ts > a works";
    let fail_b = " FAIL  test/b.test.ts > b works";
    let rows: Vec<Row> = heads
        .iter()
        .enumerate()
        .map(|(n, head)| {
            row(
                30 + n as u64,
                1,
                40 + n as u64,
                head,
                &base,
                10 + n as u32,
                vec![
                    job("test-win", "failure", &[fail_a]),
                    job("test", "failure", &[fail_b]),
                ],
            )
        })
        .collect();
    let mut rows = rows;
    rows.push(row(
        33,
        1,
        43,
        &edits_a,
        &base,
        13,
        vec![job("test-win", "failure", &[fail_a])],
    ));
    let wt_path = std::env::temp_dir().join(format!("fairlead-replay-q-wt-{}", std::process::id()));
    let run_with = |entries: &str, rows: &[Row]| {
        let _ = std::fs::remove_dir_all(&wt_path);
        let watch_windows = "[[replay.failures]]\nrunner = \"vitest\"\nextractor = \"vitest\"\njob = \"^test-win$\"\n";
        let mut config: Config =
            toml::from_str(&format!("{CONFIG}\n{watch_windows}\n{entries}")).unwrap();
        config.graph.cache = false;
        let replayer = Replayer {
            clone: &dir,
            worktree: Worktree::open(&dir, &wt_path, &base).unwrap(),
            config: &config,
            sources: Sources::new(&config).unwrap(),
        };
        let window = Window::ending("2026-09-16", 7).unwrap();
        report(
            "example/repo",
            &window,
            30,
            &replay(&replayer, rows, &window),
        )
    };
    let entry = |until: &str| {
        format!("[[replay.quarantine]]\npath = \"test/a.test.ts\"\njob = \"^test-win$\"\nreason = \"fails on Windows only\"\nuntil = \"{until}\"\n")
    };
    let active = run_with(&entry("2099-01-01"), &rows);
    assert_eq!(format!("{:?}", active.quarantine[0].status), "Active");
    assert_eq!(
        (
            active.quarantined,
            active.quarantine[0].would_miss,
            active.quarantine[0].would_hit
        ),
        (4, 3, 1),
        "pr3 edits a itself, so its failure would have been a hit"
    );
    assert_eq!(
        (active.hits, active.misses.len(), active.judged),
        (3, 0, 3),
        "b is still judged"
    );
    assert_eq!(
        active.hits_selected + active.hits_run_all + active.hits_check,
        active.hits,
        "the split counts hits only"
    );
    assert_eq!(active.recall, Some(1.0));
    assert_eq!(
        (active.raw_recall, active.raw_judged),
        (Some(4.0 / 7.0), 7),
        "raw counts a's failures as what they were"
    );
    let expired = run_with(&entry("2026-09-01"), &rows);
    assert_eq!(format!("{:?}", expired.quarantine[0].status), "Expired");
    assert_eq!(expired.misses.len(), 3);
    let too_few = run_with(&entry("2099-01-01"), &rows[..2]);
    assert_eq!(format!("{:?}", too_few.quarantine[0].status), "Unverified");
    let mut elsewhere = rows.clone();
    elsewhere[0].jobs[1] = job("test", "failure", &[fail_a]);
    let other = run_with(&entry("2099-01-01"), &elsewhere);
    assert_eq!(format!("{:?}", other.quarantine[0].status), "Unverified");
    assert_eq!(other.quarantine[0].other_jobs, ["test"]);
    let stale = run_with(
        &entry("2099-01-01").replace("test/a.test.ts", "test/none.test.ts"),
        &rows,
    );
    assert_eq!(
        format!("{:?}", stale.quarantine[0].status),
        "Unverified",
        "no failures means no pull requests"
    );
}
