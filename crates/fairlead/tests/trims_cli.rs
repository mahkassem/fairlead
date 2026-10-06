//! `fairlead plan` on changes that used to select everything: a dependency
//! version bump, Fairlead's own config, and a dispatch-only workflow, read
//! from a real git repository.

use std::path::{Path, PathBuf};
use std::process::Command;

fn plan_of(dir: &Path) -> serde_json::Value {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(["plan", "--base", "main", "--json"])
        .current_dir(dir)
        .env_clear();
    for keep in ["PATH", "SYSTEMROOT", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

const MANIFEST: &str = r#"{
  "name": "fixture",
  "private": true,
  "workspaces": ["packages/*"],
  "devDependencies": { "lint-tool": "1.0.0" }
}
"#;

const MEMBER: &str = r#"{ "name": "app", "dependencies": { "left-pad": "1.0.0" } }
"#;

const LOCK: &str = r#"{
  "lockfileVersion": 1,
  "workspaces": {
    "": {
      "name": "fixture",
      "devDependencies": {
        "lint-tool": "1.0.0",
      },
    },
    "packages/app": {
      "name": "app",
      "dependencies": {
        "left-pad": "1.0.0",
      },
    },
  },
  "packages": {
    "app": ["app@workspace:packages/app"],
    "left-pad": ["left-pad@1.0.0", "", {}, "sha512-left-pad1"],
    "lint-tool": ["lint-tool@1.0.0", "", { "bin": { "lint": "bin/lint.js" } }, "sha512-lint-tool1"],
  }
}
"#;

const NIGHTLY: &str = "on:\n  workflow_dispatch:\n  schedule:\n    - cron: '0 3 * * *'\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";

/// A committed bun workspace whose `app` package imports `left-pad`.
fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-trims-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", "[[tests.runners]]\nid = \"bun\"\nmatch = [\"**/*.test.ts\"]\ncommand = [\"bun\", \"test\", \"{files}\"]\n");
    write(&dir, "package.json", MANIFEST);
    write(&dir, "bun.lock", LOCK);
    write(&dir, "packages/app/package.json", MEMBER);
    write(
        &dir,
        "packages/app/src/pad.ts",
        "import pad from 'left-pad';\nexport const padded = pad;\n",
    );
    write(
        &dir,
        "packages/app/src/plain.ts",
        "export const plain = 1;\n",
    );
    write(
        &dir,
        "packages/app/test/pad.test.ts",
        "import { padded } from '../src/pad';\n",
    );
    write(
        &dir,
        "packages/app/test/plain.test.ts",
        "import { plain } from '../src/plain';\n",
    );
    write(&dir, ".github/workflows/nightly.yml", NIGHTLY);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// `name`'s version moved from 1.0.0 to 1.0.1 in `path`.
fn bump(dir: &Path, path: &str, name: &str) {
    let text = std::fs::read_to_string(dir.join(path)).unwrap();
    let text = text
        .replace(
            &format!("\"{name}\": \"1.0.0\""),
            &format!("\"{name}\": \"1.0.1\""),
        )
        .replace(&format!("{name}@1.0.0"), &format!("{name}@1.0.1"))
        .replace(&format!("sha512-{name}1"), &format!("sha512-{name}2"));
    write(dir, path, &text);
}

fn tests(plan: &serde_json::Value) -> Vec<&str> {
    plan["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap())
        .collect()
}

fn codes(plan: &serde_json::Value) -> Vec<&str> {
    plan["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect()
}

#[test]
fn a_bumped_dev_dependency_nothing_imports_runs_no_tests() {
    let dir = project("dev-bump");
    bump(&dir, "package.json", "lint-tool");
    bump(&dir, "bun.lock", "lint-tool");
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], false);
    assert!(tests(&plan).is_empty(), "{plan}");
    assert_eq!(codes(&plan), ["version-bump-scoped", "version-bump-scoped"]);
}

#[test]
fn a_bumped_dependency_runs_the_tests_of_the_files_importing_it() {
    let dir = project("dep-bump");
    bump(&dir, "packages/app/package.json", "left-pad");
    bump(&dir, "bun.lock", "left-pad");
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], false);
    assert_eq!(tests(&plan), ["packages/app/test/pad.test.ts"]);
    assert_eq!(
        plan["tests"][0]["reason"]["chain"][0],
        "packages/app/package.json (left-pad)"
    );
}

#[test]
fn a_config_edit_and_a_dispatch_only_workflow_run_no_tests() {
    let dir = project("config-workflow");
    write(
        &dir,
        "fairlead.toml",
        &std::fs::read_to_string(dir.join("fairlead.toml"))
            .unwrap()
            .replace("bun\"\nmatch", "bun\"\n# unit tests\nmatch"),
    );
    write(
        &dir,
        ".github/workflows/nightly.yml",
        &NIGHTLY.replace("echo hi", "echo hello"),
    );
    let plan = plan_of(&dir);
    assert_eq!(plan["all"], false);
    assert!(tests(&plan).is_empty(), "{plan}");
    assert_eq!(codes(&plan), ["workflow-dispatch-only", "fairlead-config"]);
    write(
        &dir,
        ".github/workflows/nightly.yml",
        &NIGHTLY.replace("  schedule:", "  push:\n  schedule:"),
    );
    assert_eq!(plan_of(&dir)["all"], true);
}
