use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(dir: &Path, program: &str, args: &[&str]) -> Output {
    // A clean environment, so a developer's FAIRLEAD_*, CI or git settings can't leak in.
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

fn guard(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let mut all = vec!["guard", "check"];
    all.extend_from_slice(args);
    let out = run(dir, env!("CARGO_BIN_EXE_fairlead"), &all);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

/// A repository with `fairlead.toml` and the given files, all committed.
fn repo(name: &str, config: &str, files: &[(&str, String)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-guard-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "start"]);
    dir
}

const RATCHETED: &str = "[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 3\n";
const ZERO: &str = "[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 3\nratchet = false\n";

#[test]
fn with_no_rules_configured_the_check_passes_and_says_so() {
    let dir = repo("none", "", &[("src/a.ts", lines(9))]);
    let (ok, out, _) = guard(&dir, &[]);
    assert!(ok);
    assert!(out.contains("no rules configured"), "{out}");
}

#[test]
fn a_zero_tolerance_finding_fails_the_check_and_is_printed() {
    let dir = repo(
        "zero",
        ZERO,
        &[("src/a.ts", lines(4)), ("src/b.ts", lines(3))],
    );
    let (ok, _, err) = guard(&dir, &[]);
    assert!(!ok);
    assert!(
        err.contains("src/a.ts:1 file-length: file is 4 lines, over 3"),
        "{err}"
    );
    assert!(!err.contains("src/b.ts"), "{err}");
}

#[test]
fn a_ratcheted_finding_fails_until_the_baseline_holds_it_and_a_new_one_fails_again() {
    let dir = repo("ratchet", RATCHETED, &[("src/a.ts", lines(4))]);
    let (ok, _, err) = guard(&dir, &[]);
    assert!(!ok);
    assert!(
        err.contains("src/a.ts file-length: 0 allowed, 1 found"),
        "{err}"
    );

    let (ok, out, _) = guard(&dir, &["--write-baseline"]);
    assert!(ok, "{out}");
    let written = std::fs::read_to_string(dir.join("fairlead-baseline.json")).unwrap();
    assert!(written.contains("\"file-length\": 1"), "{written}");

    let (ok, out, _) = guard(&dir, &[]);
    assert!(ok);
    assert!(
        out.contains("clean, 1 finding(s) held at the baseline"),
        "{out}"
    );

    std::fs::write(dir.join("src/b.ts"), lines(5)).unwrap();
    git(&dir, &["add", "src/b.ts"]);
    let (ok, _, err) = guard(&dir, &[]);
    assert!(!ok);
    assert!(
        err.contains("src/b.ts file-length: 0 allowed, 1 found"),
        "{err}"
    );
    assert!(!err.contains("src/a.ts"), "{err}");
}

#[test]
fn list_prints_every_finding_including_held_ones() {
    let dir = repo("list", RATCHETED, &[("src/a.ts", lines(4))]);
    assert!(guard(&dir, &["--write-baseline"]).0);
    let (ok, out, _) = guard(&dir, &["--list"]);
    assert!(ok);
    assert_eq!(
        out.lines().next(),
        Some("src/a.ts:1 file-length: file is 4 lines, over 3")
    );
}

#[test]
fn untracked_and_excluded_files_are_not_read() {
    let config = format!("{ZERO}[guard]\nexclude = [\"src/gen/**\"]\n");
    let dir = repo("scope", &config, &[("src/gen/a.ts", lines(9))]);
    std::fs::write(dir.join("src/untracked.ts"), lines(9)).unwrap();
    let (ok, out, err) = guard(&dir, &[]);
    assert!(ok, "{err}");
    assert!(out.contains("clean"), "{out}");
}

#[test]
fn staged_fails_only_on_what_the_commit_adds_and_logs_each_run() {
    let dir = repo(
        "staged",
        ZERO,
        &[("src/long.ts", lines(5)), ("src/ok.ts", lines(1))],
    );
    let log = dir.join(".git/fairlead/events.jsonl");

    // An edit inside the long file that doesn't grow it adds nothing.
    std::fs::write(
        dir.join("src/long.ts"),
        lines(5).replace("line 2", "line two"),
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, out, err) = guard(&dir, &["--staged"]);
    assert!(ok, "{err}");
    assert!(
        out.contains("1 staged file(s), the staged changes add no findings"),
        "{out}"
    );

    // Growing it, or making another file too long, adds a finding each.
    std::fs::write(dir.join("src/long.ts"), lines(6)).unwrap();
    std::fs::write(dir.join("src/ok.ts"), lines(4)).unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, _, err) = guard(&dir, &["--staged"]);
    assert!(!ok);
    assert!(err.contains("add 2 finding(s)"), "{err}");
    assert!(
        err.contains("src/long.ts:1 file-length: file is 6 lines"),
        "{err}"
    );

    // The working tree doesn't count: only the index does.
    git(&dir, &["reset", "-q"]);
    let (ok, out, _) = guard(&dir, &["--staged"]);
    assert!(ok);
    assert!(out.contains("0 staged file(s)"), "{out}");

    let events: Vec<serde_json::Value> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let decisions: Vec<&str> = events
        .iter()
        .map(|e| e["decision"].as_str().unwrap())
        .collect();
    assert_eq!(decisions, ["allow", "deny", "allow"]);
    assert_eq!(events[1]["stage"], "commit");
    assert_eq!(events[1]["added"], 2);
    assert_eq!(events[1]["rules"][0], "file-length");
}

#[test]
fn a_renamed_file_is_judged_against_its_old_self() {
    let dir = repo("rename", ZERO, &[("src/long.ts", lines(5))]);
    git(&dir, &["mv", "src/long.ts", "src/longer.ts"]);
    let (ok, out, err) = guard(&dir, &["--staged"]);
    assert!(ok, "{err}");
    assert!(out.contains("add no findings"), "{out}");
}

#[test]
fn before_the_first_commit_everything_staged_is_new() {
    let dir = std::env::temp_dir().join(format!("fairlead-guard-unborn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), ZERO).unwrap();
    std::fs::write(dir.join("src/a.ts"), lines(4)).unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, _, err) = guard(&dir, &["--staged"]);
    assert!(!ok);
    assert!(err.contains("src/a.ts:1 file-length"), "{err}");
}

#[test]
fn deny_any_fails_a_commit_on_every_finding_in_a_file_it_touches() {
    let config = format!("{ZERO}[guard]\ndeny = \"any\"\n");
    let dir = repo("deny-any", &config, &[("src/long.ts", lines(5))]);
    std::fs::write(
        dir.join("src/long.ts"),
        lines(5).replace("line 2", "line two"),
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, _, err) = guard(&dir, &["--staged"]);
    assert!(!ok);
    assert!(err.contains("the staged files have 1 finding(s)"), "{err}");
}

#[test]
fn events_off_writes_no_log() {
    let config = format!("{ZERO}[guard]\nevents = \"off\"\n");
    let dir = repo("events-off", &config, &[("src/ok.ts", lines(1))]);
    std::fs::write(dir.join("src/ok.ts"), lines(2)).unwrap();
    git(&dir, &["add", "-A"]);
    assert!(guard(&dir, &["--staged"]).0);
    assert!(!dir.join(".git/fairlead/events.jsonl").exists());
}
