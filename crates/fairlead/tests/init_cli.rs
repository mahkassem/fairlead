//! `fairlead init` on synthetic repositories: the runners it writes are the
//! ones the manifests and lockfiles show, the config it writes loads and
//! plans, and it never replaces a config unless asked.

use std::path::{Path, PathBuf};
use std::process::Command;

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
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
    cmd
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

/// A git repository holding `files`, each `(path, content)`.
fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-init-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (path, content) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let out = command(&dir, "git", &["init", "-q", "-b", "main"])
        .output()
        .unwrap();
    assert!(out.status.success());
    command(&dir, "git", &["add", "-A"]).output().unwrap();
    command(&dir, "git", &["commit", "-q", "-m", "base"])
        .output()
        .unwrap();
    dir
}

#[test]
fn a_vitest_project_on_pnpm_gets_a_runner_through_pnpm_exec_and_a_config_that_plans() {
    let dir = repo(
        "vitest",
        &[
            ("package.json", r#"{"devDependencies": {"vitest": "^3"}}"#),
            ("pnpm-lock.yaml", "lockfileVersion: '9.0'\n"),
            ("src/a.ts", "export const a = 1;\n"),
            ("src/a.test.ts", "import { a } from './a';\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("vitest"), "{out}");
    assert!(out.contains("1 test file"), "{out}");
    let toml = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    assert!(
        toml.contains(r#"command = ["pnpm", "exec", "vitest", "run", "{files}"]"#),
        "{toml}"
    );
    let (code, out) = fairlead(&dir, &["config", "check"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = fairlead(&dir, &["plan", "--files", "src/a.ts"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("src/a.test.ts"), "{out}");
}

#[test]
fn go_python_and_rust_in_one_repository_get_a_runner_each_and_a_check_for_rust() {
    let dir = repo(
        "polyglot",
        &[
            ("go.mod", "module example.com/app\n\ngo 1.22\n"),
            ("cart/cart.go", "package cart\n"),
            ("cart/cart_test.go", "package cart\n"),
            ("pyproject.toml", "[project]\nname = \"app\"\n"),
            ("uv.lock", "version = 1\n"),
            ("app/core.py", "X = 1\n"),
            ("tests/test_core.py", "from app import core\n"),
            ("Cargo.toml", "[package]\nname = \"tool\"\n"),
            ("src/main.rs", "fn main() {}\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(r#"command = ["go", "test", "{packages}"]"#),
        "{out}"
    );
    assert!(
        out.contains(r#"command = ["uv", "run", "pytest", "{files}"]"#),
        "{out}"
    );
    assert!(out.contains(r#"id = "cargo-test""#), "{out}");
    assert!(
        out.contains(r#"match = { replace = ["**/*_test.go", "**/test_*.py"] }"#),
        "{out}"
    );
    assert!(
        !dir.join("fairlead.toml").exists(),
        "--dry-run writes nothing"
    );
}

#[test]
fn a_runner_one_npm_run_away_is_found() {
    let dir = repo(
        "indirect",
        &[
            (
                "package.json",
                r#"{"scripts": {"test": "npm run lint && npm run unit", "unit": "node --test"}}"#,
            ),
            ("lib/a.js", "module.exports = 1;\n"),
            ("lib/a.test.js", "require('./a');\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(r#"command = ["node", "--test", "{files}"]"#),
        "{out}"
    );
}

#[test]
fn a_runner_with_no_test_files_is_left_out_with_a_note() {
    let dir = repo(
        "notests",
        &[
            ("package.json", r#"{"devDependencies": {"jest": "^30"}}"#),
            ("go.mod", "module example.com/app\n"),
            ("main.go", "package main\n"),
            ("main_test.go", "package main\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains(r#"id = "jest""#), "{out}");
    assert!(out.contains(r#"id = "go""#), "{out}");
    let (_, out) = fairlead(&dir, &["init"]);
    assert!(
        out.contains("note: jest: jest in package.json, but no test file matches"),
        "{out}"
    );
}

#[test]
fn an_existing_config_is_kept_unless_force_is_given() {
    let dir = repo(
        "existing",
        &[
            ("go.mod", "module example.com/app\n"),
            ("a_test.go", "package a\n"),
            ("fairlead.toml", "# mine\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("already exists"), "{out}");
    assert_eq!(
        std::fs::read_to_string(dir.join("fairlead.toml")).unwrap(),
        "# mine\n"
    );
    let (code, out) = fairlead(&dir, &["init", "--force"]);
    assert_eq!(code, 0, "{out}");
    assert!(std::fs::read_to_string(dir.join("fairlead.toml"))
        .unwrap()
        .contains(r#"id = "go""#));
}

#[test]
fn a_repository_init_knows_nothing_about_gets_no_config() {
    let dir = repo("unknown", &[("notes.txt", "hello\n")]);
    let (code, out) = fairlead(&dir, &["init"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("found no test runner init knows here"),
        "{out}"
    );
    assert!(!dir.join("fairlead.toml").exists());
}

#[test]
fn test_files_no_runner_covers_stay_out_so_the_plan_still_runs() {
    let dir = repo(
        "stray",
        &[
            ("pyproject.toml", "[project]\nname = \"p\"\n"),
            ("pkg/a.py", "x = 1\n"),
            ("tests/test_a.py", "from pkg import a\n"),
            ("js_tests/core.test.js", "test('x', () => {});\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = fairlead(&dir, &["plan", "--files", "pkg/a.py"]);
    assert_eq!(
        code, 0,
        "a JavaScript test no runner covers must not stop the plan: {out}"
    );
    assert!(!out.contains("js_tests"), "{out}");
}

#[test]
fn phpunit_test_directories_come_from_its_config() {
    let dir = repo(
        "phpdirs",
        &[
            (
                "composer.json",
                r#"{ "require-dev": { "phpunit/phpunit": "^10" } }"#,
            ),
            (
                "phpunit.xml.dist",
                r#"<phpunit><testsuites><testsuite name="unit"><directory>./Tests/</directory><directory suffix="Spec.php">./specs</directory></testsuite></testsuites></phpunit>"#,
            ),
            ("Tests/Command/RunTest.php", "<?php\n"),
            ("specs/RunSpec.php", "<?php\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(r#"match = ["Tests/**/*Test.php", "specs/**/*Spec.php"]"#),
        "{out}"
    );
    assert!(out.contains("2 test files"), "{out}");
}

#[test]
fn test_files_that_import_the_runner_are_found_whatever_they_are_called() {
    let dir = repo(
        "importing",
        &[
            (
                "package.json",
                r#"{ "devDependencies": { "vitest": "^3" } }"#,
            ),
            ("src/addDays/index.ts", "export const addDays = 1;\n"),
            (
                "src/addDays/test.ts",
                "import { it } from \"vitest\";\nimport { addDays } from \"./index\";\n",
            ),
            ("src/subDays/test.ts", "import { it } from 'vitest';\n"),
            ("src/only/testing.ts", "import { it } from 'vitest';\n"),
            ("src/helpers/test.ts", "export const notATest = 1;\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init"]);
    assert_eq!(code, 0, "{out}");
    let config = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    assert!(config.contains(r#""**/test.ts""#), "{config}");
    assert!(config.contains(r#""src/only/testing.ts""#), "{config}");
    let (code, out) = fairlead(&dir, &["plan", "--files", "src/addDays/index.ts"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("src/addDays/test.ts"), "{out}");
}

#[test]
fn ava_and_minitest_are_recognised() {
    let dir = repo(
        "ava",
        &[
            (
                "package.json",
                r#"{ "devDependencies": { "ava": "^6" }, "ava": { "files": ["test/*.js"] } }"#,
            ),
            ("test/a.js", "import test from 'ava';\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(r#"command = ["npx", "ava", "{files}"]"#),
        "{out}"
    );
    assert!(out.contains(r#"match = ["test/*.js"]"#), "{out}");

    let dir = repo(
        "minitest",
        &[
            ("Gemfile", "source 'https://rubygems.org'\n"),
            ("Rakefile", "task default: :test\n"),
            ("lib/app.rb", "class App; end\n"),
            ("test/app_test.rb", "require 'minitest/autorun'\n"),
        ],
    );
    let (code, out) = fairlead(&dir, &["init", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(r#"command = ["bundle", "exec", "rake", "test"]"#),
        "{out}"
    );
}

fn vite(name: &str) -> PathBuf {
    repo(
        name,
        &[
            (
                "package.json",
                r#"{"devDependencies": {"vitest": "^3", "vite": "^6"}}"#,
            ),
            ("vite.config.ts", "export default {};\n"),
            ("src/App.tsx", "export const App = () => null;\n"),
            ("src/App.test.tsx", "import { App } from './App';\n"),
        ],
    )
}

#[test]
fn a_blueprint_adds_its_config_files_and_workflow_and_init_alone_names_it() {
    let dir = vite("bp-vite");
    let (code, out) = fairlead(&dir, &["init"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("this looks like vite-react: `fairlead init --blueprint vite-react --force`"),
        "{out}"
    );
    assert!(
        !dir.join(".claude/skills/vite-react/SKILL.md").exists(),
        "a suggestion writes nothing"
    );
    let (code, out) = fairlead(&dir, &["init", "--blueprint", "vite-react", "--force"]);
    assert_eq!(code, 0, "{out}");
    let toml = std::fs::read_to_string(dir.join("fairlead.toml")).unwrap();
    assert!(
        toml.contains("# From the vite-react blueprint.\n[stages]"),
        "{toml}"
    );
    assert!(dir.join(".claude/skills/vite-react/SKILL.md").is_file());
    assert!(dir.join(".github/workflows/fairlead.yml").is_file());
    let (code, out) = fairlead(&dir, &["config", "check"]);
    assert_eq!(code, 0, "{out}");
    std::fs::write(dir.join(".claude/skills/vite-react/SKILL.md"), "ours\n").unwrap();
    std::fs::remove_file(dir.join("fairlead.toml")).unwrap();
    let (code, out) = fairlead(&dir, &["init", "--blueprint", "vite-react"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("kept .claude/skills/vite-react/SKILL.md"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".claude/skills/vite-react/SKILL.md")).unwrap(),
        "ours\n"
    );
}

#[test]
fn a_blueprint_whose_config_doesnt_load_leaves_no_config_behind() {
    let dir = vite("bp-bad");
    std::fs::write(
        dir.join("ours.toml"),
        "name = \"ours\"\ndescription = \"x\"\nconfig = '''\nextends = [\"nope\"]\n'''\n",
    )
    .unwrap();
    let (code, out) = fairlead(&dir, &["init", "--blueprint", "ours.toml"]);
    assert_eq!(code, 2, "{out}");
    assert!(!dir.join("fairlead.toml").exists(), "{out}");
    assert!(!dir.join(".fairlead.init.toml").exists());
    let (code, out) = fairlead(&dir, &["init", "--blueprint", "nope"]);
    assert_eq!(code, 2);
    assert!(
        out.contains("neither a built-in blueprint (laravel-api, vite-react)"),
        "{out}"
    );
}
