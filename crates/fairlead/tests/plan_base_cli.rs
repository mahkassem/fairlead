//! What `plan`, `done` and `next` compare a change with when no `--base`
//! is given: the merge base with `[pr] base`, else the default branch, else
//! HEAD with one line saying so. Committed, uncommitted and untracked
//! changes all count.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fairlead_in(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.output().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

const RUNNER: &str =
    "[[tests.runners]]\nid = \"unit\"\nmatch = [\"test/**\"]\ncommand = [\"true\", \"{files}\"]\n";

fn repo(name: &str, branch: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-base-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", branch]);
    write(&dir, "fairlead.toml", config);
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(&dir, "src/b.ts", "export const b = 1;\n");
    write(&dir, "test/a.test.ts", "import { a } from '../src/a';\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn changed(out: &Output) -> Vec<String> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    plan["changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_branch_is_planned_from_its_merge_base_with_the_pr_base_not_the_default_branch() {
    let config = format!("{RUNNER}\n[pr]\nbase = \"develop\"\nup_to_date = false\n");
    let dir = repo("pr", "main", &config);
    git(&dir, &["checkout", "-q", "-b", "develop"]);
    write(&dir, "src/b.ts", "export const b = 2;\n");
    git(&dir, &["commit", "-q", "-am", "develop's own change"]);
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    write(&dir, "src/a.ts", "export const a = 2;\n");
    git(&dir, &["commit", "-q", "-am", "the feature, committed"]);
    write(&dir, "test/c.test.ts", "export {};\n");
    let found = changed(&fairlead_in(&dir, &["plan", "--json"]));
    assert_eq!(found, ["src/a.ts", "test/c.test.ts"], "{found:?}");
    let given = changed(&fairlead_in(&dir, &["plan", "--json", "--base", "main"]));
    assert!(given.contains(&"src/b.ts".to_string()), "{given:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_no_base_branch_the_plan_says_so_once_and_covers_the_changes_since_head() {
    let dir = repo("none", "trunk", RUNNER);
    write(&dir, "src/a.ts", "export const a = 3;\n");
    let out = fairlead_in(&dir, &["plan", "--json"]);
    assert_eq!(changed(&out), ["src/a.ts"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(err.matches("no base branch found").count(), 1, "{err}");
    let done = fairlead_in(&dir, &["done", "--dry-run"]);
    let text = String::from_utf8_lossy(&done.stdout);
    assert!(done.status.success(), "{text}");
    assert!(text.contains("test/a.test.ts"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
