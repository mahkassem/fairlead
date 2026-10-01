//! `[[quarantine]]` on a real repository: a CRLF checkout under a path with
//! a space. A test held here is reported as not provable here and runs alone;
//! only the failure its entry expects is excused, and only until its date.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(dir: &Path, program: &str, args: &[&str]) -> Output {
    // A clean environment, so a developer's FAIRLEAD_*, CI or git settings can't leak in.
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT", "PATHEXT"] {
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
    cmd.output().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = run(dir, "git", args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = run(dir, env!("CARGO_BIN_EXE_fairlead"), args);
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

/// The OS the tests run on, and one they don't.
fn oses() -> (&'static str, &'static str) {
    let here = std::env::consts::OS;
    (here, if here == "linux" { "windows" } else { "linux" })
}

/// A runner that fails a test when `docs/fail-<name>.md` exists, printing
/// what it holds; `plan.ignore` keeps the marker out of the plan.
const RUNNER: &str = r#"for f; do b=$(basename "$f" .test.ts); if [ -f "docs/fail-$b.md" ]; then cat "docs/fail-$b.md"; st=1; fi; done; exit ${st:-0}"#;

fn config(until: &str) -> String {
    let (here, other) = oses();
    format!(
        r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", '{RUNNER}', "sh", "{{files}}"]

[[quarantine]]
path = "test/a.test.ts"
os = "{here}"
when = ["autocrlf", "space-in-path"]
signature = "spawnSync git ENOENT"
reason = "it spawns git in a file URL's pathname"
proved_in = "CI on Linux"
until = "{until}"

[[quarantine]]
path = "test/b.test.ts"
os = "{other}"
signature = "ENOENT"
reason = "it opens a path that only {other} spells that way"
proved_in = "CI on {here}"
until = "{until}"

[done]
guard = false
"#
    )
}

fn repo(name: &str, until: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("fairlead quarantine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("test")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "core.autocrlf", "true"]);
    std::fs::write(dir.join("fairlead.toml"), config(until)).unwrap();
    std::fs::write(dir.join("a.ts"), "export const a = 1;\n").unwrap();
    for t in ["a", "b", "c"] {
        let text = "import { a } from '../a';\n";
        std::fs::write(dir.join(format!("test/{t}.test.ts")), text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("a.ts"), "export const a = 2;\n").unwrap();
    dir
}

#[test]
fn a_held_test_is_not_provable_here_and_runs_alone_while_one_held_elsewhere_runs() {
    let dir = repo("plan", "2099-12-31");
    let (here, other) = oses();
    let (code, out) = fairlead(&dir, &["plan", "--base", "main"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("not provable here")
            && out.contains(&format!("test/a.test.ts  [{here}, autocrlf, space-in-path] it spawns git in a file URL's pathname; proved in CI on Linux, until 2099-12-31")),
        "{out}"
    );
    assert!(
        out.contains("sh test/b.test.ts test/c.test.ts\n")
            && out.contains("sh test/a.test.ts  (quarantined)"),
        "the held test runs alone, the rest together: {out}"
    );
    let (_, json) = fairlead(&dir, &["plan", "--base", "main", "--json"]);
    let plan: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(plan["quarantined"].as_array().unwrap().len(), 1, "{json}");
    assert_eq!(plan["quarantined"][0]["target"], "test/a.test.ts");
    assert_eq!(plan["invocations"][1]["quarantined"], "test/a.test.ts");
    let (_, why) = fairlead(
        &dir,
        &["test", "--base", "main", "--explain", "test/b.test.ts"],
    );
    assert!(
        why.contains(&format!("doesn't hold here ({here}, autocrlf, space-in-path): it names {other}, and this is {here}")),
        "{why}"
    );
    let (code, check) = fairlead(&dir, &["config", "check"]);
    assert_eq!(code, 0, "{check}");
    assert!(
        check.contains(&format!("here: {here}, autocrlf, space-in-path"))
            && check.contains("quarantine test/a.test.ts: holds here"),
        "{check}"
    );
}

#[test]
fn done_excuses_only_the_failure_the_entry_expects() {
    let dir = repo("done", "2099-12-31");
    std::fs::write(dir.join("docs/fail-a.md"), "Error: spawnSync git ENOENT\n").unwrap();
    let (code, out) = fairlead(&dir, &["done", "--base", "main"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("done: not provable here: unit (test/a.test.ts)")
            && out.contains("1 failed as quarantined"),
        "{out}"
    );
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    let last: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(last["decision"], "pass");
    assert_eq!(last["steps"][1]["quarantined"], true, "{last}");
    std::fs::write(dir.join("docs/fail-a.md"), "AssertionError: 1 !== 2\n").unwrap();
    let (code, out) = fairlead(&dir, &["done", "--base", "main"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("test/a.test.ts failed, and the failure counts: the output doesn't match"),
        "{out}"
    );
    std::fs::write(dir.join("docs/fail-c.md"), "spawnSync git ENOENT\n").unwrap();
    std::fs::remove_file(dir.join("docs/fail-a.md")).unwrap();
    let (code, out) = fairlead(&dir, &["done", "--base", "main"]);
    assert_eq!(code, 1, "a test no entry holds fails as ever: {out}");
}

#[test]
fn an_entry_past_its_date_is_reported_and_its_failure_counts() {
    let dir = repo("expired", "2020-01-01");
    let (_, out) = fairlead(&dir, &["plan", "--base", "main"]);
    assert!(
        out.contains(
            "[quarantine-expired] test/a.test.ts: its [[quarantine]] entry ended on 2020-01-01"
        ) && !out.contains("not provable here"),
        "{out}"
    );
    std::fs::write(dir.join("docs/fail-a.md"), "Error: spawnSync git ENOENT\n").unwrap();
    let (code, out) = fairlead(&dir, &["done", "--base", "main"]);
    assert_eq!(code, 1, "{out}");
}

#[test]
fn ci_run_records_a_held_failure_and_the_report_says_where_it_is_proved() {
    let dir = repo("ci", "2099-12-31");
    std::fs::write(dir.join("docs/fail-a.md"), "Error: spawnSync git ENOENT\n").unwrap();
    let plan = dir.join(".git").join("plan.json");
    let results = dir.join(".git").join("results.json");
    let p = plan.to_str().unwrap();
    let r = results.to_str().unwrap();
    let (code, out) = fairlead(&dir, &["ci", "plan", "--base", "main", "--out", p]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = fairlead(&dir, &["ci", "run", "--plan", p, "--results", r]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("(1 failed as quarantined, not provable here)"),
        "{out}"
    );
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&results).unwrap()).unwrap();
    assert_eq!(saved["passed"], true);
    assert_eq!(saved["invocations"][1]["quarantined"], true, "{saved}");
    let (code, report) = fairlead(&dir, &["ci", "report", "--plan", p, "--results", r]);
    assert_eq!(code, 0, "{report}");
    assert!(
        report.contains("Not provable where the plan was made")
            && report.contains("proved in CI on Linux")
            && report.contains("failed as quarantined: not provable here"),
        "{report}"
    );
}

/// npm, pnpm and yarn install `.cmd` shims on Windows, which `Command`
/// alone can't start.
#[cfg(windows)]
#[test]
fn a_check_whose_command_is_a_cmd_shim_starts() {
    let dir = repo("shim", "2099-12-31");
    let bin = dir.join(".git").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("shim-tool.cmd"),
        "@echo shim ran with %*\r\n@exit /b 0\r\n",
    )
    .unwrap();
    let mut text = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    text.push_str(
        "\n[[checks]]\nid = \"tool\"\ncommand = [\"shim-tool\", \"x y\"]\npaths = [\"*.ts\"]\n",
    );
    std::fs::write(dir.join("fairlead.toml"), text).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .args(["done", "--base", "main", "--keep-going"])
        .current_dir(&dir)
        .env("PATH", path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("shim ran with \"x y\"") && text.contains("done: passed tool"),
        "{text}"
    );
}
