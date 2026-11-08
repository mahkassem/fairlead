//! The secret-path rule and the gitleaks preset, driven through the binary:
//! what a commit may add, what the check stage says about tracked files, and
//! that a secret's value never reaches the output.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(dir: &Path, program: &str, args: &[&str], path: Option<&Path>) -> Output {
    // A clean environment, so a developer's FAIRLEAD_*, CI or git settings can't leak in.
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    let mut dirs: Vec<PathBuf> = path.map(Path::to_path_buf).into_iter().collect();
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    cmd.env("PATH", std::env::join_paths(dirs).unwrap());
    if let Some(value) = std::env::var_os("SYSTEMROOT") {
        cmd.env("SYSTEMROOT", value);
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
    let out = run(dir, "git", args, None);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn guard(dir: &Path, args: &[&str], path: Option<&Path>) -> (bool, String) {
    let mut all = vec!["guard", "check"];
    all.extend_from_slice(args);
    let out = run(dir, env!("CARGO_BIN_EXE_fairlead"), &all, path);
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

/// A repository with the given config and files, all committed.
fn repo(name: &str, config: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-secrets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", config);
    write(&dir, "README.md", "a project\n");
    for (path, text) in files {
        write(&dir, path, text);
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "start"]);
    dir
}

#[test]
fn with_no_config_a_commit_may_not_add_a_file_named_like_a_secret() {
    let dir = repo("commit", "", &[]);
    write(&dir, ".env.production", "API_KEY=1\n");
    write(&dir, "android/app/release.jks", "keystore\n");
    write(&dir, ".env.example", "API_KEY=\n");
    write(&dir, "deploy/id_ed25519.pub", "ssh-ed25519 AAAA\n");
    git(&dir, &["add", "-A"]);
    let (ok, out) = guard(&dir, &["--staged"], None);
    assert!(!ok, "{out}");
    assert!(out.contains("add 2 finding(s)"), "{out}");
    assert!(
        out.contains(
            ".env.production:1 secret-path: adds a file named like a secret (`**/.env.*`)"
        ),
        "{out}"
    );
    assert!(
        out.contains("android/app/release.jks:1 secret-path"),
        "{out}"
    );
    assert!(!out.contains(".env.example"), "{out}");
    assert!(!out.contains("id_ed25519.pub"), "{out}");
}

#[test]
fn a_fixture_key_warns_and_an_allowed_path_or_the_rule_off_passes() {
    let dir = repo(
        "allow",
        "[guard.paths]\nallow = [\"config/dev.key\"]\n",
        &[],
    );
    write(&dir, "tests/fixtures/server.pem", "not a real key\n");
    write(&dir, "config/dev.key", "dev only\n");
    git(&dir, &["add", "-A"]);
    let (ok, out) = guard(&dir, &["--staged"], None);
    assert!(ok, "{out}");
    assert!(
        out.contains("warning: tests/fixtures/server.pem:1 secret-path"),
        "{out}"
    );
    assert!(!out.contains("config/dev.key"), "{out}");
    assert!(out.contains("2 staged file(s)"), "{out}");

    let off = repo("off", "[guard]\npaths = false\n", &[]);
    write(&off, "release.p12", "x\n");
    git(&off, &["add", "-A"]);
    let (ok, out) = guard(&off, &["--staged"], None);
    assert!(ok, "{out}");
    assert!(out.contains("no rules configured"), "{out}");
}

#[test]
fn a_renamed_file_fails_only_when_the_rename_makes_it_a_secret_name() {
    let dir = repo(
        "rename",
        "",
        &[("notes.txt", "API_KEY=1\n"), (".env", "A=1\n")],
    );
    git(&dir, &["mv", ".env", ".env.local"]);
    let (ok, out) = guard(&dir, &["--staged"], None);
    assert!(ok, "a file already named like one moving isn't new: {out}");
    git(&dir, &["mv", "notes.txt", "prod.key"]);
    let (ok, out) = guard(&dir, &["--staged"], None);
    assert!(!ok, "{out}");
    assert!(out.contains("prod.key:1 secret-path"), "{out}");
}

#[test]
fn tracked_secret_names_warn_until_ratchet_is_set_and_fail_with_it_off() {
    let files = [(".env", "A=1\n"), ("src/a.ts", "x\n")];
    let rules = "[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 100\n";
    let dir = repo("tracked", &format!("[guard.paths]\n{rules}"), &files);
    let (ok, out) = guard(&dir, &[], None);
    assert!(ok, "{out}");
    assert!(
        out.contains("warning: .env:1 secret-path: is tracked and named like a secret"),
        "{out}"
    );

    let strict = repo(
        "strict",
        &format!("[guard.paths]\nratchet = false\n{rules}"),
        &files,
    );
    let (ok, out) = guard(&strict, &[], None);
    assert!(!ok, "{out}");
    assert!(out.contains(".env:1 secret-path"), "{out}");
    assert!(!out.contains("warning: .env"), "{out}");
}

const GITLEAKS: &str = "[[guard.external]]\npreset = \"gitleaks\"\n";

#[test]
fn without_gitleaks_the_preset_notes_it_and_passes() {
    let dir = repo("no-gitleaks", GITLEAKS, &[]);
    write(&dir, "app.cfg", "token = 1\n");
    git(&dir, &["add", "-A"]);
    let empty = std::env::temp_dir().join(format!("fairlead-empty-path-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    let (ok, out) = guard(&dir, &["--staged"], Some(&empty));
    assert!(ok, "{out}");
    assert!(
        out.contains("gitleaks not installed; secret content not scanned"),
        "{out}"
    );
    let (ok, out) = guard(&dir, &[], Some(&empty));
    assert!(ok, "{out}");
    assert!(out.contains("gitleaks not installed"), "{out}");
}

/// A stand-in for gitleaks that logs its arguments and reports one leak,
/// with a value that must never be printed.
#[cfg(unix)]
fn fake_gitleaks(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = r#"#!/bin/sh
echo "$@" >> "$(dirname "$0")/args.log"
if [ "$1" = version ]; then echo 8.24.3; exit 0; fi
while [ $# -gt 0 ]; do
  if [ "$1" = --report-path ]; then shift
    printf '[{"RuleID":"generic-api-key","File":"./app.cfg","StartLine":2,"Secret":"VALUE-NEVER-SHOWN","Match":"token = VALUE-NEVER-SHOWN","Fingerprint":"app.cfg:generic-api-key:2"}]' > "$1"
  fi
  shift
done
"#;
    let path = bin.join("gitleaks");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[cfg(unix)]
#[test]
fn gitleaks_findings_fail_the_commit_and_the_check_without_the_value() {
    let dir = repo("gitleaks", GITLEAKS, &[]);
    let bin = fake_gitleaks(&dir);
    write(&dir, "app.cfg", "# config\ntoken = 1\n");
    git(&dir, &["add", "app.cfg"]);
    let (ok, out) = guard(&dir, &["--staged"], Some(&bin));
    assert!(!ok, "{out}");
    assert!(
        out.contains("app.cfg:2 gitleaks: generic-api-key: a likely secret (value not shown)"),
        "{out}"
    );
    assert!(
        out.contains("`app.cfg:generic-api-key:2` to .gitleaksignore"),
        "{out}"
    );
    assert!(!out.contains("VALUE-NEVER-SHOWN"), "{out}");

    git(&dir, &["commit", "-q", "-m", "config"]);
    let (ok, out) = guard(&dir, &[], Some(&bin));
    assert!(!ok, "{out}");
    assert!(out.contains("app.cfg:2 gitleaks"), "{out}");
    assert!(!out.contains("VALUE-NEVER-SHOWN"), "{out}");

    let args = std::fs::read_to_string(bin.join("args.log")).unwrap();
    let mut calls = args.lines();
    assert!(
        calls
            .next()
            .unwrap()
            .starts_with("git --pre-commit --staged --redact"),
        "{args}"
    );
    assert!(calls.next().unwrap().starts_with("dir --redact"), "{args}");

    let doctor = run(
        &dir,
        env!("CARGO_BIN_EXE_fairlead"),
        &["doctor"],
        Some(&bin),
    );
    let text = String::from_utf8_lossy(&doctor.stdout);
    assert!(text.contains("gitleaks: 8.24.3, for `gitleaks`"), "{text}");
}

#[test]
fn a_preset_with_a_command_too_is_a_config_problem() {
    let dir = repo(
        "both",
        "[[guard.external]]\npreset = \"gitleaks\"\ncommand = [\"x\"]\n",
        &[],
    );
    let out = run(
        &dir,
        env!("CARGO_BIN_EXE_fairlead"),
        &["config", "check"],
        None,
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("guard.external"), "{err}");
}
