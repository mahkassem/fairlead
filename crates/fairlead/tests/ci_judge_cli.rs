//! `fairlead ci run --judge` on a real repository: a failing test the merged
//! change's plan selected is planned, one it left out is an escape with the
//! owner rule that would have caught it, and `ci.escapes` decides whether
//! `ci report` fails on it.

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

fn fairlead(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = command(dir, env!("CARGO_BIN_EXE_fairlead"), args);
    cmd.envs(envs.iter().copied());
    let out = cmd.output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

/// The runner fails a test file when a `FAIL-<name>` marker sits beside the
/// config, and prints `FAIL <path>` for each, which the regex extractor reads.
const CONFIG: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "for f; do if [ -f \"FAIL-$(basename $f)\" ]; then echo \"FAIL $f\"; st=1; fi; done; exit ${st:-0}", "sh", "{files}"]

[[replay.failures]]
runner = "unit"
extractor = "regex"
pattern = '^FAIL (?P<file>\S+)'
"#;

/// A merge that changed `src/a.ts`, whose plan selected only `test/a.test.ts`,
/// and a run plan that selects both tests, as a push that runs everything would.
fn merged(name: &str, extra: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-judge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), format!("{CONFIG}{extra}")).unwrap();
    std::fs::write(dir.join(".gitignore"), "FAIL-*\n*.json\n").unwrap();
    std::fs::write(dir.join("src/a.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(dir.join("src/b.ts"), "export const b = 1;\n").unwrap();
    std::fs::write(
        dir.join("test/a.test.ts"),
        "import { a } from '../src/a';\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("test/b.test.ts"),
        "import { b } from '../src/b';\n",
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("src/a.ts"), "export const a = 2;\n").unwrap();
    let plan = |out: &str| {
        let (code, text) = fairlead(&dir, &["ci", "plan", "--base", "main", "--out", out], &[]);
        assert_eq!(code, 0, "{text}");
    };
    plan("merge.json");
    std::fs::write(dir.join("src/b.ts"), "export const b = 2;\n").unwrap();
    plan("run.json");
    std::fs::write(dir.join("FAIL-a.test.ts"), "").unwrap();
    std::fs::write(dir.join("FAIL-b.test.ts"), "").unwrap();
    dir
}

fn run(dir: &Path, envs: &[(&str, &str)]) -> (i32, String) {
    fairlead(
        dir,
        &[
            "ci",
            "run",
            "--plan",
            "run.json",
            "--judge",
            "merge.json",
            "--results",
            "results.json",
        ],
        envs,
    )
}

#[test]
fn a_failing_test_the_merge_left_out_is_an_escape_with_the_rule_that_catches_it() {
    let dir = merged("escape", "");
    let (code, out) = run(&dir, &[("GITHUB_ACTIONS", "true")]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("FAIL test/b.test.ts"),
        "the runner's output still shows: {out}"
    );
    assert!(
        out.contains("test/a.test.ts failed and the merge's plan selected it"),
        "{out}"
    );
    let rule = r#"[[tests.owners]] match = "test/**", covers = ["src/**"]"#;
    assert!(
        out.contains(&format!(
            "escape: test/b.test.ts failed and the merge's plan left it out; {rule}"
        )),
        "{out}"
    );
    assert!(
        out.contains("::warning file=test/b.test.ts,title=Fairlead escape::"),
        "{out}"
    );
    let results: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("results.json")).unwrap()).unwrap();
    let judged: Vec<(&str, &str)> = results["judged"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["test"].as_str().unwrap(), v["verdict"].as_str().unwrap()))
        .collect();
    assert_eq!(
        judged,
        [("test/a.test.ts", "planned"), ("test/b.test.ts", "escaped")]
    );
    let (code, md) = fairlead(
        &dir,
        &[
            "ci",
            "report",
            "--plan",
            "run.json",
            "--results",
            "results.json",
        ],
        &[],
    );
    assert_eq!(
        code, 0,
        "escapes are reported, not failed, by default: {md}"
    );
    assert!(md.contains("**1 escape**"), "{md}");
    assert!(
        md.contains(&format!("| `test/b.test.ts` | `{rule}` |")),
        "{md}"
    );
    assert!(
        md.contains("1 failing test the merge's plan selected"),
        "{md}"
    );
}

#[test]
fn with_escapes_fail_the_report_fails_on_an_escape() {
    let dir = merged("fail", "\n[ci]\nescapes = \"fail\"\n");
    run(&dir, &[]);
    let (code, out) = fairlead(
        &dir,
        &[
            "ci",
            "report",
            "--plan",
            "run.json",
            "--results",
            "results.json",
        ],
        &[],
    );
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("escaped the merge's plan, and ci.escapes is \"fail\""),
        "{out}"
    );
    std::fs::remove_file(dir.join("FAIL-b.test.ts")).unwrap();
    run(&dir, &[]);
    let (code, out) = fairlead(
        &dir,
        &[
            "ci",
            "report",
            "--plan",
            "run.json",
            "--results",
            "results.json",
        ],
        &[],
    );
    assert_eq!(code, 0, "a planned failure alone isn't an escape: {out}");
}

#[test]
fn a_runner_with_no_failures_entry_is_named_and_nothing_is_judged() {
    let dir = merged("unread", "");
    let config = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    let cut = config.find("[[replay.failures]]").unwrap();
    std::fs::write(dir.join("fairlead.toml"), &config[..cut]).unwrap();
    let (code, out) = run(&dir, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("no [[replay.failures]] entry for runner `unit`"),
        "{out}"
    );
    let results: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("results.json")).unwrap()).unwrap();
    assert!(results.get("judged").is_none(), "{results}");
}
