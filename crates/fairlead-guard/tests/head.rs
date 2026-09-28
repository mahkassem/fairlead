//! Whether a path is in HEAD, read from `.git`'s files, in each state a
//! repository gets into: where the files answer, the answer is git's, and
//! where they can't be sure they give none, so the caller asks git.

use std::path::{Path, PathBuf};
use std::process::Command;

use fairlead_guard::head::in_head;

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    let mut cmd = Command::new("git");
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
    cmd.output().unwrap().status.success()
}

fn git(dir: &Path, args: &[&str]) {
    assert!(git_ok(dir, args), "git {args:?}");
}

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A repository with one committed migration, `db/001.sql`.
fn repo(name: &str, init: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-head-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &[&["init", "-q", "-b", "main"], init].concat());
    write(&dir, "db/001.sql", "select 1;\n");
    write(&dir, "src/a.txt", "a\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "start"]);
    dir
}

/// The files' answer is `want`, and any answer they give is git's.
fn answers(dir: &Path, path: &str, want: Option<bool>) {
    let got = in_head(dir, path);
    assert_eq!(got, want, "{path} in {}", dir.display());
    if let Some(got) = got {
        let spec = format!("HEAD:./{path}");
        assert_eq!(got, git_ok(dir, &["cat-file", "-e", &spec]), "{path}");
    }
}

#[test]
fn after_a_commit_the_index_answers_as_git_does_in_every_index_version() {
    let dir = repo("committed", &[]);
    answers(&dir, "db/001.sql", Some(true));
    answers(&dir, "db/002.sql", Some(false));
    answers(&dir.join("db"), "001.sql", Some(true));
    for version in ["4", "3", "2"] {
        git(&dir, &["update-index", "--index-version", version]);
        answers(&dir, "db/001.sql", Some(true));
        answers(&dir, "src/a.txt", Some(true));
        answers(&dir, "db/002.sql", Some(false));
    }
}

#[test]
fn a_packed_commit_a_packed_ref_and_a_linked_worktree_are_read_too() {
    let dir = repo("packed", &[]);
    let worktree = dir.with_extension("wt");
    let _ = std::fs::remove_dir_all(&worktree);
    git(&dir, &["gc", "-q"]);
    assert!(
        !dir.join(".git/refs/heads/main").exists(),
        "the ref is packed"
    );
    answers(&dir, "db/001.sql", Some(true));
    git(
        &dir,
        &["worktree", "add", "-q", &worktree.to_string_lossy()],
    );
    answers(&worktree, "db/001.sql", Some(true));
    answers(&worktree, "db/002.sql", Some(false));
}

#[test]
fn a_staged_file_a_failed_commit_or_a_soft_reset_is_left_to_git() {
    let dir = repo("staged", &[]);
    write(&dir, "db/002.sql", "select 2;\n");
    git(&dir, &["add", "db/002.sql"]);
    answers(&dir, "db/002.sql", None);

    // A commit its hook refuses still writes the staged tree into the index.
    write(&dir, ".git/hooks/pre-commit", "#!/bin/sh\nexit 1\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hook = dir.join(".git/hooks/pre-commit");
        std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(!git_ok(&dir, &["commit", "-q", "-m", "refused"]));
    answers(&dir, "db/002.sql", None);

    std::fs::remove_file(dir.join(".git/hooks/pre-commit")).unwrap();
    git(&dir, &["commit", "-q", "-m", "second"]);
    answers(&dir, "db/002.sql", Some(true));
    git(&dir, &["reset", "-q", "--soft", "HEAD~"]);
    answers(&dir, "db/002.sql", None);
}

#[test]
fn a_split_index_is_left_to_git_and_a_sha256_repository_is_read() {
    let dir = repo("split", &[]);
    git(&dir, &["update-index", "--split-index"]);
    answers(&dir, "db/001.sql", None);

    let probe = std::env::temp_dir().join(format!("fairlead-head-probe-{}", std::process::id()));
    std::fs::create_dir_all(&probe).unwrap();
    // Older git has no SHA-256 object format; there is nothing to read.
    if git_ok(&probe, &["init", "-q", "--object-format=sha256"]) {
        let dir = repo("sha256", &["--object-format=sha256"]);
        answers(&dir, "db/001.sql", Some(true));
        answers(&dir, "db/002.sql", Some(false));
    }
}
