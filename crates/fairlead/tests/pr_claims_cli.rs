//! The done receipt on a pull request: `receipt --pr` prints a block for
//! the body, `pr check` in CI fails a body that claims the change was
//! verified unless a receipt for the head's tree passed, and `ci report`
//! folds the body's receipt into its summary, saying whose tree it is for.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run_env(dir: &Path, program: &str, args: &[&str], env: &[(&str, &str)]) -> Output {
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

fn git(dir: &Path, args: &[&str]) -> String {
    let out = run_env(dir, "git", args, &[]);
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fairlead(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let out = run_env(dir, env!("CARGO_BIN_EXE_fairlead"), args, env);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

const CONFIG: &str = "[[tests.runners]]\nid = \"unit\"\nmatch = [\"test/**\"]\ncommand = [\"sh\", \"-c\", \"exit 0\", \"sh\", \"{files}\"]\n\n[done]\nguard = false\n\n[pr]\nup_to_date = false\n";

/// A branch with one committed change on top of `main`.
fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-claims-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join("test/a.test.ts"), "export {};\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "feat/x"]);
    std::fs::write(dir.join("test/a.test.ts"), "export const a = 1;\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "change"]);
    dir
}

fn event(dir: &Path, body: &str) -> PathBuf {
    let payload = serde_json::json!({ "pull_request": {
        "title": "Change", "body": body, "labels": [],
        "base": { "ref": "main", "sha": git(dir, &["rev-parse", "main"]) },
        "head": { "ref": "feat/x", "sha": git(dir, &["rev-parse", "HEAD"]) },
    }});
    let path = dir.join(".git/event.json");
    std::fs::write(&path, payload.to_string()).unwrap();
    path
}

#[test]
fn a_claim_in_the_body_needs_a_passing_receipt_for_the_head_tree() {
    let dir = repo("ci");
    let path = event(&dir, "Fixes it. Verified locally, all tests pass.");
    let env = [("GITHUB_EVENT_PATH", path.to_str().unwrap())];
    let (ok, out) = fairlead(&dir, &["pr", "check"], &env);
    assert!(!ok, "{out}");
    assert!(
        out.contains("pr-claims: the body says \"verified\", \"all tests pass\", and no receipt for the head's tree passed"),
        "{out}"
    );

    let (ok, out) = fairlead(&dir, &["done", "--base", "main"], &[]);
    assert!(ok, "{out}");
    let (ok, block) = fairlead(&dir, &["receipt", "--base", "main", "--pr"], &[]);
    assert!(ok, "{block}");
    let tree = git(&dir, &["rev-parse", "HEAD^{tree}"]);
    assert!(
        block.starts_with(&format!(
            "<!-- fairlead-receipt tree={tree} gate=passed -->"
        )),
        "{block}"
    );
    let path = event(&dir, &format!("Verified locally.\n\n{block}"));
    let env = [("GITHUB_EVENT_PATH", path.to_str().unwrap())];
    let (ok, out) = fairlead(&dir, &["pr", "check"], &env);
    assert!(ok, "{out}");

    let plan = dir.join(".git/plan.json");
    let plan_arg = plan.to_str().unwrap();
    let (ok, out) = fairlead(
        &dir,
        &["ci", "plan", "--base", "main", "--out", plan_arg],
        &[],
    );
    assert!(ok, "{out}");
    let (ok, report) = fairlead(&dir, &["ci", "report", "--plan", plan_arg], &env);
    assert!(ok, "{report}");
    assert!(
        report.contains("<summary>Receipt for the head's tree: gate passed</summary>"),
        "{report}"
    );

    std::fs::write(dir.join("test/a.test.ts"), "export const a = 2;\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "a later push"]);
    let path = event(&dir, &format!("Verified locally.\n\n{block}"));
    let env = [("GITHUB_EVENT_PATH", path.to_str().unwrap())];
    let (ok, out) = fairlead(&dir, &["pr", "check"], &env);
    assert!(!ok, "a receipt for an earlier tree doesn't count: {out}");
    let (_, report) = fairlead(&dir, &["ci", "report", "--plan", plan_arg], &env);
    assert!(report.contains("Receipt for an earlier tree ("), "{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_body_that_claims_nothing_or_denies_it_passes_without_a_receipt() {
    let dir = repo("plain");
    let path = event(&dir, "Renames a helper. Not tested on Windows.");
    let env = [("GITHUB_EVENT_PATH", path.to_str().unwrap())];
    let (ok, out) = fairlead(&dir, &["pr", "check"], &env);
    assert!(ok, "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
