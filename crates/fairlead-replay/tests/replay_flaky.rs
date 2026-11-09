//! Replay's flaky outcome for a test that ran out of time: a timeout with no
//! assertion behind it is the runner's, not a gap in the plan.

use fairlead_core::config::Config;
use fairlead_replay::git::Worktree;
use fairlead_replay::run::{replay, Replayer, Sources};
use fairlead_replay::window::Window;

mod common;
use common::{commit, git, job, row, write, CONFIG};

#[test]
fn a_test_that_timed_out_without_an_assertion_failing_is_flaky_not_a_miss() {
    let dir = std::env::temp_dir().join(format!("fairlead-replay-timeout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "src/b.ts", "export const b = 1;\n");
    write(&dir, "test/a.test.ts", "it('a works', () => {});\n");
    write(&dir, "test/b.test.ts", "import { b } from '../src/b';\n");
    let base = commit(&dir, "base");
    git(&dir, &["checkout", "-q", "-b", "pr14"]);
    write(&dir, "src/b.ts", "export const b = 2;\n");
    let head = commit(&dir, "pr14: change b");
    git(&dir, &["checkout", "-q", "main"]);
    let failing = " FAIL  test/a.test.ts > a works";
    let rows = vec![
        row(
            1,
            1,
            14,
            &head,
            &base,
            10,
            vec![job(
                "test",
                "failure",
                &[failing, "Error: Test timed out in 5000ms."],
            )],
        ),
        row(
            2,
            1,
            14,
            &head,
            &base,
            11,
            vec![job(
                "test",
                "failure",
                &[failing, "AssertionError: expected 1 to be 2"],
            )],
        ),
    ];
    let mut config: Config = toml::from_str(CONFIG).unwrap();
    config.graph.cache = false;
    let wt_path =
        std::env::temp_dir().join(format!("fairlead-replay-timeout-wt-{}", std::process::id()));
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
    assert_eq!(outcomes, ["Flaky", "Miss"]);
}
