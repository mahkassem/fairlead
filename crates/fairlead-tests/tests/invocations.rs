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

const GO: &[(&str, &str)] = &[
    ("go.mod", "module example.com/shop\n\ngo 1.24\n"),
    (
        "pkg/price/price.go",
        "package price\n\nfunc Total(xs []int) int { return sum(xs) }\n",
    ),
    (
        "pkg/price/sum.go",
        "package price\n\nfunc sum(xs []int) (t int) { for _, x := range xs { t += x }; return }\n",
    ),
    (
        "pkg/price/price_test.go",
        "package price\n\nimport \"testing\"\n\nfunc TestTotal(t *testing.T) {}\n",
    ),
    (
        "cmd/shop/main.go",
        "package main\n\nimport (\n\t\"fmt\"\n\t\"example.com/shop/pkg/price\"\n)\n\nfunc main() { fmt.Println(price.Total(nil)) }\n",
    ),
    (
        "cmd/shop/main_test.go",
        "package main\n\nimport \"testing\"\n\nfunc TestMain(t *testing.T) {}\n",
    ),
    (
        "pkg/other/other_test.go",
        "package other\n\nimport \"testing\"\n\nfunc TestOther(t *testing.T) {}\n",
    ),
];

const GO_RUNNER: &str = r#"
[tests]
match = ["**/*_test.go"]

[plan]
run_all = ["go.sum"]

[[tests.runners]]
id = "go"
match = ["**/*_test.go"]
command = ["go", "test", "{packages}"]
"#;

#[test]
fn go_test_gets_each_selected_package_and_everything_under_run_all() {
    let mut files = GO.to_vec();
    files.push(("go.sum", "x\n"));
    let dir = repo("inv-go", &files);
    let plan = run(&dir, &config(GO_RUNNER), vec![modified("pkg/price/sum.go")]);
    assert_eq!(
        tests(&plan),
        ["cmd/shop/main_test.go", "pkg/price/price_test.go"],
        "through the package's other file and its importer"
    );
    let go = plan.invocations.iter().find(|i| i.id == "go").unwrap();
    assert_eq!(go.argv, ["go", "test", "./cmd/shop", "./pkg/price"]);
    let plan = run(&dir, &config(GO_RUNNER), vec![modified("go.sum")]);
    let go = plan.invocations.iter().find(|i| i.id == "go").unwrap();
    assert_eq!(go.argv, ["go", "test", "./..."]);
}

#[test]
fn a_go_module_manifest_selects_that_module_and_its_importers_not_everything() {
    let dir = repo(
        "inv-go-modules",
        &[
            (
                "app/go.mod",
                "module example.com/app\n\nreplace example.com/lib => ../lib\n",
            ),
            (
                "app/main.go",
                "package main\n\nimport \"example.com/lib/money\"\n\nvar _ = money.Add\n",
            ),
            (
                "app/main_test.go",
                "package main\n\nimport \"testing\"\n\nfunc TestApp(t *testing.T) {}\n",
            ),
            ("lib/go.mod", "module example.com/lib\n"),
            ("lib/go.sum", "example.com/dep v1.0.0 h1:x=\n"),
            (
                "lib/money/add.go",
                "package money\n\nfunc Add(a, b int) int { return a + b }\n",
            ),
            (
                "lib/money/add_test.go",
                "package money\n\nimport \"testing\"\n\nfunc TestAdd(t *testing.T) {}\n",
            ),
            ("tools/go.mod", "module example.com/tools\n"),
            ("tools/gen/gen.go", "package gen\n"),
            (
                "tools/gen/gen_test.go",
                "package gen\n\nimport \"testing\"\n\nfunc TestGen(t *testing.T) {}\n",
            ),
        ],
    );
    let cfg = r#"
[tests]
match = ["**/*_test.go"]
"#;
    let plan = run(&dir, &config(cfg), vec![modified("lib/go.sum")]);
    assert!(!plan.all);
    assert_eq!(tests(&plan), ["app/main_test.go", "lib/money/add_test.go"]);
}
