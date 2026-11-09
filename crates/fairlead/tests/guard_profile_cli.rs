//! The guard-only profile, the `[pr]` rules and the pre-push check, driven
//! through the binary: what `init --profile guard` writes, what doctor and
//! the score say about it, and what `pr check` and `guard check --push` catch.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run_env(dir: &Path, program: &str, args: &[&str], env: &[(&str, &str)]) -> Output {
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
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = run_env(dir, "git", args, &[]);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The binary's exit status and everything it printed.
fn fairlead(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let out = run_env(dir, env!("CARGO_BIN_EXE_fairlead"), args, env);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-gp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A repository on `main` with the given files, all committed.
fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = scratch(name);
    git(&dir, &["init", "-q", "-b", "main"]);
    for (path, text) in files {
        write(&dir, path, text);
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "start"]);
    dir
}

#[test]
fn init_with_the_guard_profile_writes_a_guard_only_config_for_the_stack() {
    let dir = repo(
        "init",
        &[
            ("Gemfile", "source 'https://rubygems.org'\n"),
            ("db/migrate/001_create_users.rb", "class CreateUsers; end\n"),
        ],
    );
    let (ok, out) = fairlead(&dir, &["init", "--profile", "guard"], &[]);
    assert!(ok, "{out}");
    assert!(out.contains("(guard-only profile)"), "{out}");
    let config = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    for line in [
        "profile = \"guard\"",
        "[guard.paths]",
        "preset = \"gitleaks\"",
        "files = [\"db/migrate/*.rb\"]",
        "base = \"origin/main\"",
        "unique_prefix = {}",
        "fetch = true",
    ] {
        assert!(config.contains(line), "{line} in:\n{config}");
    }
    assert!(!config.contains("[[tests.runners]]"), "{config}");
    let lefthook = std::fs::read_to_string(dir.join("lefthook.yml")).unwrap();
    assert!(lefthook.contains("guard check --staged"), "{lefthook}");
    assert!(
        lefthook.contains("run: fairlead done --pre-push"),
        "{lefthook}"
    );
    let (ok, out) = fairlead(&dir, &["config", "check"], &[]);
    assert!(ok, "{out}");
}

#[test]
fn doctor_and_the_score_describe_the_guard_profile() {
    let dir = repo("doctor", &[("README.md", "x\n")]);
    write(
        &dir,
        "fairlead.toml",
        "profile = \"guard\"\n[guard.paths]\n",
    );
    let (_, out) = fairlead(&dir, &["doctor"], &[]);
    assert!(
        out.contains(
            "profile: guard (no test plan; `fairlead done` runs `guard check` and `pr check`)"
        ),
        "{out}"
    );
    assert!(!out.contains("phase:"), "{out}");

    let (_, out) = fairlead(&dir, &["doctor", "--score", "--json"], &[]);
    let score: serde_json::Value = serde_json::from_str(&out).expect(&out);
    assert_eq!(score["profile"], "guard");
    let na: Vec<&str> = score["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["na"] == true)
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(na, ["runners", "replay", "recall"]);
    // Only the config item is earned: 10 of the 55 points the profile uses.
    assert_eq!(score["score"], 18);

    write(
        &dir,
        ".github/workflows/guard.yml",
        "jobs:\n  g:\n    steps:\n      - run: fairlead guard check\n",
    );
    let (_, out) = fairlead(&dir, &["doctor", "--score"], &[]);
    assert!(
        out.contains("guard profile: 20/55 on the 7 items it uses"),
        "{out}"
    );
    assert!(out.contains("[-]   n/a runners"), "{out}");
    assert!(
        out.contains("a workflow runs `fairlead guard check`"),
        "{out}"
    );
}

#[test]
fn done_in_the_guard_profile_runs_only_the_guard_and_the_pr_rules() {
    let dir = repo("done", &[("src/a.ts", "export const a = 1;\n")]);
    write(
        &dir,
        "fairlead.toml",
        "profile = \"guard\"\n[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 100\n[pr]\nbranch = '^(main|feat/.+)$'\n",
    );
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "config"]);
    let (ok, out) = fairlead(&dir, &["done"], &[]);
    assert!(ok, "{out}");
    assert!(out.contains("guard"), "{out}");
    assert!(out.contains("pr"), "{out}");
    assert!(!out.contains("no test runner"), "{out}");

    git(&dir, &["checkout", "-q", "-b", "wip"]);
    let (ok, out) = fairlead(&dir, &["done"], &[]);
    assert!(!ok, "{out}");
}

const PR: &str = "[pr]\nbase = \"main\"\nbranch = '^(feat|fix)/'\nticket = { pattern = '[A-Z]+-[0-9]+', in = [\"branch\", \"title\"], any = true }\nbody_headings = [\"Summary\"]\nlabels = { forbid = [\"wip\"] }\n[[pr.files]]\npath = [\"CHANGELOG.md\"]\nwhen = [\"src/**\"]\nunless_label = \"no-changelog\"\n";

#[test]
fn pr_check_without_rules_says_so() {
    let dir = repo("pr-none", &[("a.txt", "x\n")]);
    let (ok, out) = fairlead(&dir, &["pr", "check"], &[]);
    assert!(ok, "{out}");
    assert!(out.contains("pr: no [pr] rules configured"), "{out}");
}

#[test]
fn pr_check_locally_fails_the_branch_name_and_leaves_what_it_cant_see_unchecked() {
    let dir = repo("pr-local", &[("fairlead.toml", PR), ("src/a.ts", "1\n")]);
    git(&dir, &["checkout", "-q", "-b", "tidy-up"]);
    write(&dir, "src/a.ts", "2\n");
    git(&dir, &["commit", "-q", "-am", "change"]);
    let (ok, out) = fairlead(&dir, &["pr", "check", "--local"], &[]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("pr-branch: the branch name `tidy-up` doesn't match `^(feat|fix)/`"),
        "{out}"
    );
    assert!(out.contains("pr-ticket"), "{out}");
    // The changelog could be excused by a label, which only a pull request has.
    assert!(!out.contains("pr-file:"), "{out}");
    assert!(out.contains("not checked here:"), "{out}");
    assert!(out.contains("labels (no pull request)"), "{out}");
    assert!(
        !out.contains("couldn't fetch"),
        "no remote, nothing to fetch: {out}"
    );
}

/// A GitHub `pull_request` event for the branch, as CI hands it over.
fn event(dir: &Path, base: &str, head: &str, title: &str, body: &str, labels: &[&str]) -> PathBuf {
    let sha = |rev: &str| {
        let out = run_env(dir, "git", &["rev-parse", rev], &[]);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let payload = serde_json::json!({ "pull_request": {
        "title": title, "body": body,
        "labels": labels.iter().map(|l| serde_json::json!({ "name": l })).collect::<Vec<_>>(),
        "base": { "ref": "main", "sha": sha(base) },
        "head": { "ref": "feat/ABC-12-login", "sha": sha(head) },
    }});
    let path = dir.join(".git/event.json");
    std::fs::write(&path, payload.to_string()).unwrap();
    path
}

#[test]
fn pr_check_in_ci_reads_the_event_and_holds_every_rule() {
    let dir = repo("pr-ci", &[("fairlead.toml", PR), ("src/a.ts", "1\n")]);
    git(&dir, &["checkout", "-q", "-b", "feat/ABC-12-login"]);
    write(&dir, "src/a.ts", "2\n");
    git(&dir, &["commit", "-q", "-am", "change"]);

    let bad = event(&dir, "main", "HEAD", "Login", "no headings", &["wip"]);
    let env = [("GITHUB_EVENT_PATH", bad.to_str().unwrap())];
    let (ok, out) = fairlead(&dir, &["pr", "check"], &env);
    assert!(!ok, "{out}");
    for rule in [
        "pr-body",
        "pr-labels: the label `wip` is set",
        "pr-file: the change adds no file matching CHANGELOG.md",
    ] {
        assert!(out.contains(rule), "{rule} in {out}");
    }
    assert!(!out.contains("pr-ticket"), "the branch has the key: {out}");

    let good = event(
        &dir,
        "main",
        "HEAD",
        "ABC-12 Login",
        "## Summary\nLogin.\n",
        &["no-changelog"],
    );
    let env = [("GITHUB_EVENT_PATH", good.to_str().unwrap())];
    let (ok, out) = fairlead(&dir, &["pr", "check"], &env);
    assert!(ok, "{out}");
    assert!(out.contains("rule(s) passed"), "{out}");
}

#[test]
fn pr_check_in_ci_fails_a_branch_cut_before_the_base_moved() {
    let dir = repo("pr-stale", &[("fairlead.toml", PR), ("src/a.ts", "1\n")]);
    git(&dir, &["checkout", "-q", "-b", "feat/ABC-12-login"]);
    write(&dir, "CHANGELOG.md", "- login\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "change"]);
    git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "b.txt", "later\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "main moves on"]);
    git(&dir, &["checkout", "-q", "feat/ABC-12-login"]);
    let path = event(&dir, "main", "HEAD", "ABC-12 Login", "## Summary\n", &[]);
    let env = [("GITHUB_EVENT_PATH", path.to_str().unwrap())];
    let (ok, out) = fairlead(&dir, &["pr", "check"], &env);
    assert!(!ok, "{out}");
    assert!(out.contains("pr-stale"), "{out}");
}

const NUMBERED: &str = "[guard.migrations]\nfiles = [\"migrations/*.sql\"]\nbase = \"origin/main\"\nunique_prefix = {}\nfetch = true\n";

/// Our clone and the bare origin it pushes to, with one migration on main.
fn with_origin(name: &str) -> (PathBuf, PathBuf) {
    let top = scratch(name);
    git(&top, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
    let me = top.join("me");
    std::fs::create_dir_all(&me).unwrap();
    git(&me, &["init", "-q", "-b", "main"]);
    git(&me, &["remote", "add", "origin", "../origin.git"]);
    write(&me, "fairlead.toml", NUMBERED);
    write(&me, "migrations/001_init.sql", "create table a ();\n");
    git(&me, &["add", "-A"]);
    git(&me, &["commit", "-q", "-m", "start"]);
    git(&me, &["push", "-q", "origin", "main"]);
    git(&me, &["fetch", "-q", "origin"]);
    (top, me)
}

#[test]
fn guard_check_push_fetches_the_base_and_finds_a_number_taken_since() {
    let (top, me) = with_origin("push");
    git(&top, &["clone", "-q", "origin.git", "them"]);
    let them = top.join("them");
    write(&them, "migrations/002_theirs.sql", "create table b ();\n");
    git(&them, &["add", "-A"]);
    git(&them, &["commit", "-q", "-m", "theirs"]);
    git(&them, &["push", "-q", "origin", "main"]);

    git(&me, &["checkout", "-q", "-b", "feat/x"]);
    write(&me, "migrations/002_mine.sql", "create table c ();\n");
    git(&me, &["add", "-A"]);
    git(&me, &["commit", "-q", "-m", "mine"]);
    let (ok, out) = fairlead(&me, &["guard", "check", "--push"], &[]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("migrations/002_mine.sql:1 migration-prefix: shares its number 002 with 002_theirs.sql on origin/main"),
        "{out}"
    );

    // Offline, the local ref is used with a warning; it already has theirs.
    git(&me, &["remote", "set-url", "origin", "../nowhere.git"]);
    let (ok, out) = fairlead(&me, &["guard", "check", "--push"], &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("couldn't fetch origin main"), "{out}");
    assert!(out.contains("002_theirs.sql"), "{out}");
}

#[test]
fn guard_check_push_passes_a_free_number_and_says_nothing_to_do_without_rules() {
    let (_, me) = with_origin("push-free");
    git(&me, &["checkout", "-q", "-b", "feat/y"]);
    write(&me, "migrations/002_mine.sql", "create table c ();\n");
    git(&me, &["add", "-A"]);
    git(&me, &["commit", "-q", "-m", "mine"]);
    let (ok, out) = fairlead(&me, &["guard", "check", "--push"], &[]);
    assert!(ok, "{out}");
    assert!(
        out.contains("migration numbers are free on the base"),
        "{out}"
    );

    let plain = repo("push-none", &[("a.txt", "x\n")]);
    let (ok, out) = fairlead(&plain, &["guard", "check", "--push"], &[]);
    assert!(ok, "{out}");
    assert!(out.contains("no rules configured"), "{out}");
}
