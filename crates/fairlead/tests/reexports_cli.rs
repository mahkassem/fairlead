//! A change behind a barrel selects only the tests that import the names it
//! declares, end to end through `fairlead plan` on a real git repository.

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

const RUNNER: &str = "[[tests.runners]]\nid = \"vitest\"\nmatch = [\"**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n";

/// A workspace package whose root is a barrel of `export *` lines, and a
/// test for each of two of the names it re-exports.
fn project(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fairlead-reexports-cli-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", &format!("{RUNNER}{config}"));
    write(
        &dir,
        "package.json",
        r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    );
    write(
        &dir,
        "packages/core/package.json",
        r#"{ "name": "core", "exports": { ".": "./src/index.ts" } }"#,
    );
    write(
        &dir,
        "packages/core/src/index.ts",
        "export * from './money';\nexport * from './dates';\n",
    );
    write(
        &dir,
        "packages/core/src/money.ts",
        "export const money = 1;\n",
    );
    write(
        &dir,
        "packages/core/src/dates.ts",
        "export const today = 1;\n",
    );
    write(&dir, "packages/app/package.json", r#"{ "name": "app" }"#);
    write(
        &dir,
        "packages/app/test/money.test.ts",
        "import { money } from 'core';\n",
    );
    write(
        &dir,
        "packages/app/test/dates.test.ts",
        "import { today } from 'core';\n",
    );
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn selected(dir: &Path) -> Vec<String> {
    let out = fairlead_in(dir, &["plan", "--base", "main", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["all"], false);
    plan["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_change_behind_a_barrel_selects_only_the_test_importing_its_name() {
    let dir = project("narrow", "");
    write(
        &dir,
        "packages/core/src/money.ts",
        "export const money = 2;\n",
    );
    assert_eq!(selected(&dir), ["packages/app/test/money.test.ts"]);
    let why = fairlead_in(
        &dir,
        &[
            "test",
            "--base",
            "main",
            "--explain",
            "packages/app/test/dates.test.ts",
        ],
    );
    let why = String::from_utf8_lossy(&why.stdout);
    assert!(
        why.contains("reaches packages/core/src/money.ts only through what packages/core/src/index.ts re-exports"),
        "{why}"
    );
}

#[test]
fn a_change_to_the_barrel_itself_selects_every_importer() {
    let dir = project("barrel", "");
    write(
        &dir,
        "packages/core/src/index.ts",
        "export * from './money';\nexport * from './dates';\nexport const VERSION = 2;\n",
    );
    assert_eq!(
        selected(&dir),
        [
            "packages/app/test/dates.test.ts",
            "packages/app/test/money.test.ts"
        ]
    );
}

#[test]
fn turned_off_a_change_behind_a_barrel_selects_every_importer() {
    let dir = project("off", "\n[graph]\nreexports = false\n");
    write(
        &dir,
        "packages/core/src/money.ts",
        "export const money = 2;\n",
    );
    assert_eq!(
        selected(&dir),
        [
            "packages/app/test/dates.test.ts",
            "packages/app/test/money.test.ts"
        ]
    );
}

#[test]
fn a_change_to_a_module_that_does_something_on_load_selects_every_importer() {
    let dir = project("effects", "");
    let effectful = "dayjs.extend(utc);\nexport const today = 1;\n";
    write(&dir, "packages/core/src/dates.ts", effectful);
    git(&dir, &["commit", "-q", "-am", "effect"]);
    write(
        &dir,
        "packages/core/src/dates.ts",
        &format!("{effectful}// edited\n"),
    );
    assert_eq!(
        selected(&dir),
        [
            "packages/app/test/dates.test.ts",
            "packages/app/test/money.test.ts"
        ]
    );
}
