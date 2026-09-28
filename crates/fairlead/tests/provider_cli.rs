//! An external graph provider on a real repository: its edges select tests
//! the built-in scanner can't see, and a provider that fails runs everything.

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

/// A graph as a Go build tool might print it: the test depends on a.go,
/// which depends on b.go. It checks it was given the files it claims.
const GRAPH: &str = r#"grep -q pkg/b.go && printf '{"version":1,"edges":[{"from":"pkg/a_test.go","to":"pkg/a.go"},{"from":"pkg/a.go","to":"pkg/b.go"},{"from":"pkg/a.go","to":"missing.go"}]}'"#;

fn project(name: &str, command: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-provider-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    let config = format!(
        "[tests]\nmatch = [\"**/*_test.go\"]\n\n[[tests.runners]]\nid = \"go\"\nmatch = [\"**/*_test.go\"]\ncommand = [\"go\", \"test\", \"{{files}}\"]\n\n[[graph.providers]]\nid = \"go\"\ncommand = [\"sh\", \"-c\", {command:?}]\nfiles = [\"**/*.go\"]\n"
    );
    write(&dir, "fairlead.toml", &config);
    write(&dir, "pkg/a.go", "package pkg\n");
    write(&dir, "pkg/b.go", "package pkg\n");
    write(&dir, "pkg/a_test.go", "package pkg\n");
    write(&dir, "pkg/other_test.go", "package pkg\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    write(&dir, "pkg/b.go", "package pkg // changed\n");
    dir
}

fn plan(dir: &Path) -> serde_json::Value {
    let out = fairlead_in(dir, &["plan", "--base", "main", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn a_providers_edges_select_the_tests_that_reach_a_change() {
    let dir = project("ok", GRAPH);
    let plan = plan(&dir);
    let tests: Vec<&str> = plan["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap())
        .collect();
    assert_eq!(tests, ["pkg/a_test.go"], "{plan}");
    assert_eq!(plan["all"], false);
    let out = fairlead_in(&dir, &["graph", "stats"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("providers: typescript 0 edges from 0 files; go 2 edges from 4 files"),
        "{text}"
    );
    assert!(
        text.contains("go printed 1 edges naming a file outside the tree"),
        "{text}"
    );
    assert!(
        text.contains("provider 2"),
        "edges of the provider's own kind: {text}"
    );
}

#[cfg(unix)]
#[test]
fn a_provider_that_runs_past_its_timeout_fails_instead_of_hanging_the_plan() {
    let dir = project("slow", "sleep 30");
    let config = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    std::fs::write(dir.join("fairlead.toml"), config + "timeout_seconds = 1\n").unwrap();
    let started = std::time::Instant::now();
    let plan = plan(&dir);
    assert!(
        started.elapsed().as_secs() < 10,
        "it waited {:?}",
        started.elapsed()
    );
    assert_eq!(plan["all"], true);
    assert!(plan.to_string().contains("timeout"), "{plan}");
}

#[test]
fn a_provider_that_fails_runs_every_test_and_says_why() {
    let dir = project("fail", "echo broken >&2; exit 3");
    let plan = plan(&dir);
    assert_eq!(plan["all"], true, "{plan}");
    let warning = plan["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["code"] == "provider-failed")
        .expect("a provider-failed warning");
    assert!(
        warning["message"].as_str().unwrap().contains("broken"),
        "{warning}"
    );
}
