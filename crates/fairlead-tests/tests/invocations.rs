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
    std::fs::write(
        dir.join("go.work"),
        "go 1.24\n\nuse (\n\t./app\n\t./lib\n\t./tools\n)\n",
    )
    .unwrap();
    let plan = run(&dir, &config(cfg), vec![modified("lib/go.sum")]);
    assert_eq!(
        tests(&plan),
        [
            "app/main_test.go",
            "lib/money/add_test.go",
            "tools/gen/gen_test.go"
        ],
        "a workspace picks versions across its modules"
    );
}

const JVM: &[(&str, &str)] = &[
    (
        "core/src/main/java/com/acme/Order.java",
        "package com.acme;\npublic class Order {}\n",
    ),
    (
        "core/src/main/kotlin/com/acme/Price.kt",
        "package com.acme\n\ndata class Price(val cents: Long)\n",
    ),
    (
        "core/src/test/java/com/acme/OrderTest.java",
        "package com.acme;\nclass OrderTest { Order o; }\n",
    ),
    (
        "core/src/test/kotlin/com/acme/PriceTest.kt",
        "package com.acme\n\nclass PriceTest { val p = Price(1) }\n",
    ),
    ("build.gradle", "plugins { id 'java' }\n"),
];

const JVM_RUNNERS: &str = r#"
[tests]
match = ["**/src/test/**"]

[plan]
run_all = ["build.gradle"]

[[tests.runners]]
id = "gradle"
match = ["**/src/test/**"]
command = ["./gradlew", "test", "--tests={class}"]
"#;

#[test]
fn jvm_runners_get_class_names_repeated_or_joined_and_none_when_everything_runs() {
    let dir = repo("inv-jvm", JVM);
    let plan = run(
        &dir,
        &config(JVM_RUNNERS),
        vec![
            modified("core/src/main/java/com/acme/Order.java"),
            modified("core/src/main/kotlin/com/acme/Price.kt"),
        ],
    );
    let argv = |id: &str| {
        plan.invocations
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.argv.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        argv("gradle"),
        [
            "./gradlew",
            "test",
            "--tests=com.acme.OrderTest",
            "--tests=com.acme.PriceTest"
        ]
    );
    let maven = JVM_RUNNERS.replace(
        "id = \"gradle\"\nmatch = [\"**/src/test/**\"]\ncommand = [\"./gradlew\", \"test\", \"--tests={class}\"]",
        "id = \"maven\"\nmatch = [\"**/src/test/**\"]\ncommand = [\"mvn\", \"test\", \"-Dtest={classes}\"]",
    );
    assert!(maven.contains("mvn"));
    let plan = run(
        &dir,
        &config(&maven),
        vec![
            modified("core/src/main/java/com/acme/Order.java"),
            modified("core/src/main/kotlin/com/acme/Price.kt"),
        ],
    );
    assert_eq!(
        plan.invocations[0].argv,
        [
            "mvn",
            "test",
            "-Dtest=com.acme.OrderTest,com.acme.PriceTest"
        ]
    );

    let plan = run(&dir, &config(JVM_RUNNERS), vec![modified("build.gradle")]);
    let gradle = plan.invocations.iter().find(|i| i.id == "gradle").unwrap();
    assert_eq!(gradle.argv, ["./gradlew", "test"]);
}

const WHOLE_RUNNERS: &str = r#"
[[tests.runners]]
id = "vitest"
match = ["packages/**"]
command = ["vitest", "run", "{files}"]
all_command = ["vitest", "run", "--project", "unit"]

[[tests.runners]]
id = "jest"
match = ["tools/**"]
invoke = "per-module"
cwd = "{module}"
command = ["jest", "--selectProjects", "{module.id}", "{files}"]
all_command = ["jest", "--selectProjects", "{module.id}", "--rootDir", "{module}"]
"#;

const HELD: &str = r#"
[[quarantine]]
path = "packages/a/test/x.test.ts"
signature = "ENOENT"
reason = "spawns a process from a URL path"
proved_in = "CI on Linux"
until = "2099-12-31"

[[quarantine]]
path = "tools/gen/test/g.test.ts"
signature = "ENOENT"
reason = "spawns a process from a URL path"
proved_in = "CI on Linux"
until = "2099-12-31"
"#;

fn argvs<'a>(plan: &'a fairlead_core::plan::Plan, id: &str) -> Vec<(&'a [String], bool)> {
    plan.invocations
        .iter()
        .filter(|i| i.id == id)
        .map(|i| (i.argv.as_slice(), i.quarantined.is_some()))
        .collect()
}

fn run_all_repo(name: &str) -> std::path::PathBuf {
    let mut files = FILES.to_vec();
    files.push(("pnpm-lock.yaml", "x\n"));
    repo(name, &files)
}

#[test]
fn all_command_runs_when_everything_does_and_command_otherwise() {
    let dir = run_all_repo("inv-all-command");
    let plan = run(
        &dir,
        &config(WHOLE_RUNNERS),
        vec![modified("pnpm-lock.yaml")],
    );
    assert_eq!(
        argvs(&plan, "vitest"),
        [(
            &["vitest", "run", "--project", "unit"].map(String::from)[..],
            false
        )]
    );
    let jest = ["jest", "--selectProjects", "gen", "--rootDir", "tools/gen"].map(String::from);
    assert_eq!(argvs(&plan, "jest"), [(&jest[..], false)]);

    let plan = run(
        &dir,
        &config(WHOLE_RUNNERS),
        vec![
            modified("packages/a/src/x.ts"),
            modified("tools/gen/src/g.ts"),
        ],
    );
    let vitest = [
        "vitest",
        "run",
        "packages/a/test/x.test.ts",
        "packages/b/test/y.test.ts",
    ]
    .map(String::from);
    assert_eq!(argvs(&plan, "vitest"), [(&vitest[..], false)]);
    let jest = ["jest", "--selectProjects", "gen", "test/g.test.ts"].map(String::from);
    assert_eq!(argvs(&plan, "jest"), [(&jest[..], false)]);
}

#[test]
fn exclude_arg_keeps_everything_whole_and_runs_each_held_test_alone() {
    let dir = run_all_repo("inv-exclude");
    let text = WHOLE_RUNNERS
        .replace(
            "all_command = [\"vitest\", \"run\", \"--project\", \"unit\"]",
            "all_command = [\"vitest\", \"run\", \"--project\", \"unit\"]\nexclude_arg = [\"--exclude\", \"{file}\"]",
        )
        .replace(
            "\"--rootDir\", \"{module}\"]",
            "\"--rootDir\", \"{module}\"]\nexclude_arg = [\"--testPathIgnorePatterns={file}\"]",
        )
        + HELD;
    let plan = run(&dir, &config(&text), vec![modified("pnpm-lock.yaml")]);
    let whole = [
        "vitest",
        "run",
        "--project",
        "unit",
        "--exclude",
        "packages/a/test/x.test.ts",
    ]
    .map(String::from);
    let alone = ["vitest", "run", "packages/a/test/x.test.ts"].map(String::from);
    assert_eq!(
        argvs(&plan, "vitest"),
        [(&whole[..], false), (&alone[..], true)]
    );
    let whole = [
        "jest",
        "--selectProjects",
        "gen",
        "--rootDir",
        "tools/gen",
        "--testPathIgnorePatterns=test/g.test.ts",
    ]
    .map(String::from);
    let alone = ["jest", "--selectProjects", "gen", "test/g.test.ts"].map(String::from);
    assert_eq!(
        argvs(&plan, "jest"),
        [(&whole[..], false), (&alone[..], true)]
    );
    assert!(
        plan.warnings
            .iter()
            .all(|w| w.code != "quarantine-narrowed-everything"),
        "{:?}",
        plan.warnings
    );
}

#[test]
fn without_exclude_arg_everything_names_the_runners_tests_and_warns() {
    let dir = run_all_repo("inv-narrowed");
    let text = WHOLE_RUNNERS.to_string() + HELD;
    let plan = run(&dir, &config(&text), vec![modified("pnpm-lock.yaml")]);
    let named = ["vitest", "run", "packages/b/test/y.test.ts"].map(String::from);
    let alone = ["vitest", "run", "packages/a/test/x.test.ts"].map(String::from);
    assert_eq!(
        argvs(&plan, "vitest"),
        [(&named[..], false), (&alone[..], true)]
    );
    let narrowed: Vec<_> = plan
        .warnings
        .iter()
        .filter(|w| w.code == "quarantine-narrowed-everything")
        .collect();
    assert_eq!(narrowed.len(), 2, "{:?}", plan.warnings);
    assert_eq!(
        narrowed[0].path.as_deref(),
        Some("packages/a/test/x.test.ts")
    );
    assert!(
        narrowed[0].message.contains("runner `vitest`")
            && narrowed[0].message.contains("exclude_arg"),
        "{}",
        narrowed[0].message
    );

    let plan = run(&dir, &config(&text), vec![modified("packages/a/src/x.ts")]);
    assert!(
        plan.warnings
            .iter()
            .all(|w| w.code != "quarantine-narrowed-everything"),
        "only a plan that runs everything is narrowed: {:?}",
        plan.warnings
    );
}
