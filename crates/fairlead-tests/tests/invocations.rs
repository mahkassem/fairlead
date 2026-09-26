//! Checks and invocations: what the plan tells CI to run.

mod common;

use common::*;
use fairlead_core::plan::{InvocationKind, Reason};

const FILES: &[(&str, &str)] = &[
    (
        "package.json",
        r#"{ "workspaces": ["packages/*", "tools/*"] }"#,
    ),
    ("packages/a/package.json", r#"{ "name": "a" }"#),
    ("packages/a/src/x.ts", "export const x = 1;\n"),
    (
        "packages/a/test/x.test.ts",
        "import { x } from '../src/x';\n",
    ),
    ("packages/b/package.json", r#"{ "name": "b" }"#),
    ("packages/b/src/y.ts", "import { x } from 'a';\n"),
    ("packages/b/test/y.test.ts", "import '../src/y';\n"),
    ("tools/gen/package.json", r#"{ "name": "gen" }"#),
    ("tools/gen/src/g.ts", "export const g = 1;\n"),
    ("tools/gen/test/g.test.ts", "import '../src/g';\n"),
    ("packages/a/src/old.ts", "export const old = 1;\n"),
];

const RUNNERS: &str = r#"
[[tests.runners]]
id = "vitest"
match = ["packages/**"]
command = ["vitest", "run", "{files}"]

[[tests.runners]]
id = "jest"
match = ["tools/**"]
invoke = "per-module"
cwd = "{module}"
command = ["jest", "--selectProjects", "{module.id}", "{files}"]

[[checks]]
id = "typecheck"
command = ["tsc", "-b"]
paths = ["**/*.ts"]

[[checks]]
id = "lint"
command = ["eslint", "{files}"]
paths = ["**/*.ts"]
files = "changed"

[[checks]]
id = "gen-contract"
command = ["gen", "check"]
modules = ["gen"]

[[checks]]
id = "secrets"
command = ["scan-secrets"]
"#;

#[test]
fn a_once_runner_gets_one_invocation_with_every_selected_file() {
    let dir = repo("inv-once", FILES);
    let plan = run(
        &dir,
        &config(RUNNERS),
        vec![modified("packages/a/src/x.ts")],
    );
    let vitest = plan.invocations.iter().find(|i| i.id == "vitest").unwrap();
    assert_eq!(vitest.cwd, ".");
    assert_eq!(
        vitest.argv,
        [
            "vitest",
            "run",
            "packages/a/test/x.test.ts",
            "packages/b/test/y.test.ts"
        ]
    );
    assert!(plan.invocations.iter().all(|i| i.id != "jest"));
}

#[test]
fn a_per_module_runner_runs_from_each_module_with_relative_files() {
    let dir = repo("inv-module", FILES);
    let plan = run(&dir, &config(RUNNERS), vec![modified("tools/gen/src/g.ts")]);
    let jest: Vec<_> = plan.invocations.iter().filter(|i| i.id == "jest").collect();
    assert_eq!(jest.len(), 1);
    assert_eq!(jest[0].cwd, "tools/gen");
    assert_eq!(
        jest[0].argv,
        ["jest", "--selectProjects", "gen", "test/g.test.ts"]
    );
}

#[test]
fn under_run_all_each_runner_runs_whole_and_per_module_runners_once_per_module() {
    let mut files = FILES.to_vec();
    files.push(("pnpm-lock.yaml", "x\n"));
    let dir = repo("inv-all", &files);
    let plan = run(&dir, &config(RUNNERS), vec![modified("pnpm-lock.yaml")]);
    let vitest = plan.invocations.iter().find(|i| i.id == "vitest").unwrap();
    assert_eq!(vitest.argv, ["vitest", "run"]);
    let jest: Vec<_> = plan.invocations.iter().filter(|i| i.id == "jest").collect();
    assert_eq!(jest.len(), 1);
    assert_eq!(jest[0].argv, ["jest", "--selectProjects", "gen"]);
    let lint = plan.invocations.iter().find(|i| i.id == "lint").unwrap();
    assert!(
        lint.argv.len() > 1,
        "files = changed expands as matched under run_all"
    );
}

#[test]
fn checks_match_deleted_paths_but_never_pass_them_as_files() {
    let dir = repo("inv-checks", FILES);
    let plan = run(
        &dir,
        &config(RUNNERS),
        vec![
            modified("packages/a/src/x.ts"),
            deleted("packages/a/src/gone.ts"),
        ],
    );
    let ids: Vec<&str> = plan.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["typecheck", "lint", "secrets"]);
    assert_eq!(plan.checks[0].reason, Reason::Paths { matched: 2 });
    assert_eq!(plan.checks[2].reason, Reason::Always);
    let lint = plan.invocations.iter().find(|i| i.id == "lint").unwrap();
    assert_eq!(lint.kind, InvocationKind::Check);
    assert_eq!(lint.argv, ["eslint", "packages/a/src/x.ts"]);
}

#[test]
fn a_check_watching_a_module_runs_when_the_change_reaches_it() {
    let dir = repo("inv-modules", FILES);
    let cfg = config(&RUNNERS.replace("paths = [\"**/*.ts\"]", "paths = [\"**/*.never\"]"));
    let plan = run(&dir, &cfg, vec![modified("tools/gen/src/g.ts")]);
    let gen = plan.checks.iter().find(|c| c.id == "gen-contract").unwrap();
    assert_eq!(
        gen.reason,
        Reason::Modules {
            modules: vec!["gen".into()]
        }
    );
    let other = run(&dir, &cfg, vec![modified("packages/b/src/y.ts")]);
    assert!(other.checks.iter().all(|c| c.id != "gen-contract"));
}
