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
            vec![job("test", "failure", &[fail_b])],
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
        "run 1's b passed at run 4, whose change reaches nothing"
    );
    assert_eq!(r.unavailable, 1, "run 6's commit isn't in the clone");
    assert_eq!(r.unattributed, 1, "run 7 names no file");
    assert_eq!(r.errors, 1, "run 10's commit has a test two runners match");
    assert_eq!(r.recall, Some(0.5));
}
