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
fn findings_all_fails_a_commit_on_every_finding_in_a_file_it_touches() {
    let config = format!("{ZERO}[guard]\nfindings = \"all\"\n");
    let dir = repo("findings-all", &config, &[("src/long.ts", lines(5))]);
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

#[test]
fn on_finding_warn_shows_what_a_commit_adds_and_lets_it_through() {
    let config = format!("{ZERO}[guard]\non_finding = \"warn\"\n");
    let dir = repo("warn", &config, &[("src/ok.ts", lines(1))]);
    std::fs::write(dir.join("src/ok.ts"), lines(4)).unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, _, err) = guard(&dir, &["--staged"]);
    assert!(ok, "{err}");
    assert!(err.contains("add 1 finding(s), committing anyway"), "{err}");
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    assert!(log.contains("\"decision\":\"warn\""), "{log}");
}

#[test]
fn a_config_in_a_subdirectory_checks_its_files_and_logs_to_the_repository() {
    let dir = repo(
        "subdir",
        "",
        &[
            ("app/fairlead.toml", ZERO.to_string()),
            ("app/src/ok.ts", lines(1)),
        ],
    );
    let app = dir.join("app");
    std::fs::write(app.join("src/ok.ts"), lines(4)).unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, _, err) = guard(&app, &["--staged"]);
    assert!(!ok);
    assert!(err.contains("src/ok.ts:1 file-length"), "{err}");
    assert!(dir.join(".git/fairlead/events.jsonl").exists());
}

const MIGRATIONS: &str = "[guard.migrations]\nfiles = [\"db/*.sql\"]\nunique_prefix = { allow = [[\"002_a.sql\", \"002_b.sql\"]] }\n";

#[test]
fn a_commit_may_add_a_migration_but_not_change_one_that_exists() {
    let dir = repo(
        "mig-commit",
        MIGRATIONS,
        &[("db/001_init.sql", "create table a (id int);\n".into())],
    );
    std::fs::write(dir.join("db/003_next.sql"), "create table c (id int);\n").unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, out, err) = guard(&dir, &["--staged"]);
    assert!(ok, "{out}{err}");

    std::fs::write(dir.join("db/001_init.sql"), "create table a (id bigint);\n").unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, _, err) = guard(&dir, &["--staged"]);
    assert!(!ok);
    assert!(
        err.contains("db/001_init.sql:1 migration-edit: changes a migration that existed at HEAD"),
        "{err}"
    );
}

#[test]
fn a_commit_that_gives_two_migrations_one_number_fails_unless_they_are_allowed() {
    let dir = repo(
        "mig-prefix",
        MIGRATIONS,
        &[("db/001_init.sql", "select 1;\n".into())],
    );
    for name in ["002_a.sql", "002_b.sql"] {
        std::fs::write(dir.join("db").join(name), "select 2;\n").unwrap();
    }
    git(&dir, &["add", "-A"]);
    assert!(guard(&dir, &["--staged"]).0);
    std::fs::write(dir.join("db/001_again.sql"), "select 3;\n").unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, _, err) = guard(&dir, &["--staged"]);
    assert!(!ok);
    assert!(
        err.contains(
            "db/001_again.sql:1 migration-prefix: shares its number 001 with 001_init.sql"
        ),
        "{err}"
    );
}

#[test]
fn the_check_stage_compares_migrations_with_the_base_it_is_given() {
    let dir = repo(
        "mig-base",
        MIGRATIONS,
        &[("db/001_init.sql", "select 1;\n".into())],
    );
    git(&dir, &["checkout", "-q", "-b", "work"]);
    std::fs::write(dir.join("db/001_init.sql"), "select 2;\n").unwrap();
    git(&dir, &["commit", "-qam", "edit"]);
    let (ok, _, err) = guard(&dir, &[]);
    assert!(ok, "{err}");
    assert!(err.contains("aren't checked here"), "{err}");
    let (ok, _, err) = guard(&dir, &["--base", "main"]);
    assert!(!ok);
    assert!(
        err.contains("db/001_init.sql:1 migration-edit: changes a migration that existed at main"),
        "{err}"
    );
}

#[test]
fn test_names_and_citations_run_at_the_check_stage() {
    let config = concat!(
        "[guard.test_names]\nfiles = [\"test/**\"]\nfile = '^[a-z-]+\\.test\\.ts$'\ntitles_without = 'T[0-9]{4}'\n",
        "[guard.citations]\nfiles = [\"src/**\"]\npattern = '\\((?P<code>T[0-9]{4})\\)'\nheadings_in = \"LESSONS.md\"\n",
    );
    let dir = repo(
        "names-cite",
        config,
        &[
            ("LESSONS.md", "## T1024\n".into()),
            (
                "test/Leave.test.ts",
                "test(\"T1024 works\", () => {})\n".into(),
            ),
            (
                "src/a.ts",
                "// Why (T1024).\n// And why (T4040).\nexport const a = 1\n".into(),
            ),
        ],
    );
    let (ok, _, err) = guard(&dir, &[]);
    assert!(!ok);
    assert!(err.contains("test/Leave.test.ts:1 test-file-name"), "{err}");
    assert!(
        err.contains("test/Leave.test.ts:1 test-title: test title carries \"T1024\""),
        "{err}"
    );
    assert!(
        err.contains("src/a.ts:1 citation: cites \"T4040\""),
        "{err}"
    );
    assert!(!err.contains("\"T1024\", which"), "{err}");
}

#[cfg(unix)]
#[test]
fn an_external_rule_runs_at_its_stages_and_its_findings_count() {
    let config = "[[guard.external]]\nid = \"lint\"\ncommand = [\"sh\", \"-c\", \"echo 'src/a.ts:2 no-raw-color: use a token'; exit 1\"]\n";
    let dir = repo("external", config, &[("src/a.ts", lines(3))]);
    let (ok, _, err) = guard(&dir, &[]);
    assert!(!ok);
    assert!(
        err.contains("src/a.ts:2 lint: no-raw-color: use a token"),
        "{err}"
    );
    std::fs::write(dir.join("src/a.ts"), lines(4)).unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, out, err) = guard(&dir, &["--staged"]);
    assert!(ok, "{out}{err}");
}

#[test]
fn a_migration_base_that_cannot_be_found_falls_back_to_head_at_commit() {
    let config = format!("{MIGRATIONS}base = \"origin/missing\"\n");
    let dir = repo(
        "mig-nobase",
        &config,
        &[("db/001_init.sql", "select 1;\n".into())],
    );
    std::fs::write(dir.join("db/001_init.sql"), "select 2;\n").unwrap();
    git(&dir, &["add", "-A"]);
    let (ok, _, err) = guard(&dir, &["--staged"]);
    assert!(!ok);
    assert!(
        err.contains("changes a migration that existed at HEAD"),
        "{err}"
    );
}
