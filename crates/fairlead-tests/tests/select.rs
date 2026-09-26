//! One scenario per selection rule, each on its own synthetic repository.

mod common;

use common::*;
use fairlead_core::plan::Reason;

const WORKSPACE: &[(&str, &str)] = &[
    (
        "package.json",
        r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    ),
    (
        "packages/core/package.json",
        r#"{ "name": "core", "exports": { ".": "./src/index.ts" } }"#,
    ),
    ("packages/core/src/index.ts", "export * from './money';\n"),
    ("packages/core/src/money.ts", "export const money = 1;\n"),
    ("packages/core/src/unused.ts", "export const unused = 1;\n"),
    (
        "packages/core/test/money.test.ts",
        "import { money } from '../src/money';\n",
    ),
    ("packages/billing/package.json", r#"{ "name": "billing" }"#),
    (
        "packages/billing/src/invoice.ts",
        "import { money } from 'core';\nexport const invoice = money;\n",
    ),
    (
        "packages/billing/test/invoice.test.ts",
        "import { invoice } from '../src/invoice';\n",
    ),
    ("packages/billing/test/other.test.ts", "export {};\n"),
    ("packages/docs/package.json", r#"{ "name": "docs" }"#),
    ("packages/docs/src/site.ts", "export const site = 1;\n"),
];

#[test]
fn a_change_selects_every_test_that_depends_on_it_with_the_chain() {
    let dir = repo("import", WORKSPACE);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/core/src/money.ts")],
    );
    assert_eq!(
        tests(&plan),
        [
            "packages/billing/test/invoice.test.ts",
            "packages/core/test/money.test.ts"
        ]
    );
    assert_eq!(
        reason(&plan, "packages/billing/test/invoice.test.ts"),
        &Reason::Import {
            chain: vec![
                "packages/core/src/money.ts".into(),
                "packages/core/src/index.ts".into(),
                "packages/billing/src/invoice.ts".into(),
                "packages/billing/test/invoice.test.ts".into(),
            ]
        }
    );
    assert!(!plan.all);
}

#[test]
fn a_changed_test_file_selects_itself() {
    let dir = repo("changed-test", WORKSPACE);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/billing/test/other.test.ts")],
    );
    assert_eq!(tests(&plan), ["packages/billing/test/other.test.ts"]);
    assert_eq!(
        reason(&plan, "packages/billing/test/other.test.ts"),
        &Reason::Changed
    );
}

#[test]
fn a_run_all_path_selects_everything() {
    let mut files = WORKSPACE.to_vec();
    files.push(("pnpm-lock.yaml", "lockfileVersion: 9\n"));
    let dir = repo("run-all", &files);
    let plan = run(&dir, &config(VITEST), vec![modified("pnpm-lock.yaml")]);
    assert!(plan.all);
    assert_eq!(plan.tests.len(), 3);
    assert_eq!(
        reason(&plan, "packages/billing/test/other.test.ts"),
        &Reason::RunAll {
            path: "pnpm-lock.yaml".into()
        }
    );
}

#[test]
fn a_changed_package_manifest_counts_as_every_file_in_the_package_changing() {
    let dir = repo("manifest", WORKSPACE);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/billing/package.json")],
    );
    assert_eq!(
        tests(&plan),
        [
            "packages/billing/test/invoice.test.ts",
            "packages/billing/test/other.test.ts"
        ]
    );
    match reason(&plan, "packages/billing/test/other.test.ts") {
        Reason::Import { chain } => assert_eq!(chain[0], "packages/billing/package.json"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_owner_rule_claims_tests_by_the_segment_the_change_captured() {
    let files = [
        ("services/api/src/orders.ts", "export const orders = 1;\n"),
        (
            "services/api/test/integration/orders.test.ts",
            "export {};\n",
        ),
        (
            "services/web/test/integration/pages.test.ts",
            "export {};\n",
        ),
    ];
    let dir = repo("owners", &files);
    let cfg = config(&format!(
        "{VITEST}\n[[tests.owners]]\nmatch = \"services/{{name}}/test/integration/**\"\ncovers = [\"services/{{name}}/src/**\"]\n"
    ));
    let plan = run(&dir, &cfg, vec![modified("services/api/src/orders.ts")]);
    assert_eq!(
        tests(&plan),
        ["services/api/test/integration/orders.test.ts"]
    );
    assert!(matches!(
        reason(&plan, "services/api/test/integration/orders.test.ts"),
        Reason::Owner { rule: 0, .. }
    ));
    assert_eq!(plan.unreached[0].selected, "owner");
}

#[test]
fn canaries_always_run_demand_tests_only_when_changed_and_own_tests_only_by_owner() {
    let files = [
        ("src/a.ts", "export const a = 1;\n"),
        ("e2e/smoke.spec.ts", "export {};\n"),
        ("e2e/visual.spec.ts", "import { a } from '../src/a';\n"),
        ("contract/a.test.ts", "import { a } from '../src/a';\n"),
    ];
    let dir = repo("classes", &files);
    let cfg = config(&format!(
        "{VITEST}\n[[tests.classes]]\nclass = \"canary\"\nmatch = [\"e2e/smoke.spec.ts\"]\n[[tests.classes]]\nclass = \"demand\"\nmatch = [\"e2e/visual.spec.ts\"]\n[[tests.classes]]\nclass = \"own\"\nmatch = [\"contract/**\"]\n"
    ));
    let plan = run(&dir, &cfg, vec![modified("src/a.ts")]);
    assert_eq!(tests(&plan), ["e2e/smoke.spec.ts"]);
    assert_eq!(reason(&plan, "e2e/smoke.spec.ts"), &Reason::Canary);
    let changed = run(&dir, &cfg, vec![modified("e2e/visual.spec.ts")]);
    assert!(tests(&changed).contains(&"e2e/visual.spec.ts"));
}

#[test]
fn an_unreached_file_in_a_module_with_tests_selects_that_modules_tests() {
    let dir = repo("unreached-module", WORKSPACE);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/core/src/unused.ts")],
    );
    assert_eq!(tests(&plan), ["packages/core/test/money.test.ts"]);
    assert_eq!(
        reason(&plan, "packages/core/test/money.test.ts"),
        &Reason::Unreached {
            path: "packages/core/src/unused.ts".into(),
            policy: "module".into()
        }
    );
    assert_eq!(plan.unreached[0].selected, "module");
    assert!(plan.warnings.iter().any(|w| w.code == "unreached"));
}

#[test]
fn an_unreached_file_at_the_root_or_in_a_module_without_tests_selects_everything() {
    let mut files = WORKSPACE.to_vec();
    files.push(("scripts/setup.ts", "export {};\n"));
    let dir = repo("unreached-all", &files);
    let root = run(&dir, &config(VITEST), vec![modified("scripts/setup.ts")]);
    assert!(root.all);
    assert_eq!(root.tests.len(), 3);
    let testless = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/docs/src/site.ts")],
    );
    assert!(testless.all);
    assert_eq!(testless.unreached[0].selected, "all");
}

#[test]
fn a_change_whose_dependents_reach_no_test_is_unreached() {
    let files = [
        ("package.json", r#"{ "workspaces": ["packages/*"] }"#),
        ("packages/app/package.json", r#"{ "name": "app" }"#),
        ("packages/app/src/lib.ts", "export const lib = 1;\n"),
        ("packages/app/src/main.ts", "import { lib } from './lib';\n"),
        ("packages/app/test/other.test.ts", "export {};\n"),
    ];
    let dir = repo("dead-end", &files);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/app/src/lib.ts")],
    );
    assert_eq!(tests(&plan), ["packages/app/test/other.test.ts"]);
    assert_eq!(plan.unreached[0].path, "packages/app/src/lib.ts");
}

#[test]
fn under_the_warn_policy_an_unreached_file_selects_nothing_but_is_listed() {
    let dir = repo("unreached-warn", WORKSPACE);
    let cfg = config(
        &format!("{VITEST}\n[tests]\nunreached = \"warn\"\n")
            .replace("[[tests.runners]]", "[[tests.runners]]"),
    );
    let plan = run(&dir, &cfg, vec![modified("packages/core/src/unused.ts")]);
    assert!(plan.tests.is_empty());
    assert_eq!(plan.unreached[0].selected, "none");
}

#[test]
fn an_ignored_path_selects_nothing_unless_something_references_it() {
    let mut files = WORKSPACE.to_vec();
    files.push(("README.md", "# Readme\n"));
    files.push((
        "packages/core/test/fixture.test.ts",
        "const doc = 'docs/api.md';\n",
    ));
    files.push(("docs/api.md", "# API\n"));
    let dir = repo("ignore", &files);
    let readme = run(&dir, &config(VITEST), vec![modified("README.md")]);
    assert!(readme.tests.is_empty() && !readme.all);
    assert_eq!(readme.ignored, ["README.md"]);
    let referenced = run(&dir, &config(VITEST), vec![modified("docs/api.md")]);
    assert_eq!(tests(&referenced), ["packages/core/test/fixture.test.ts"]);
    assert!(referenced.ignored.is_empty());
}

#[test]
fn a_changeset_note_selects_nothing() {
    let mut files = WORKSPACE.to_vec();
    files.push((".changeset/quick-fox.md", "---\n'core': patch\n---\nFix.\n"));
    let dir = repo("changeset", &files);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified(".changeset/quick-fox.md")],
    );
    assert!(
        plan.tests.is_empty() && !plan.all,
        "a release note isn't read by tests"
    );
    assert_eq!(plan.ignored, [".changeset/quick-fox.md"]);
}

#[test]
fn a_non_literal_dynamic_import_at_the_root_depends_on_every_file() {
    let mut files = WORKSPACE.to_vec();
    files.push(("tests/plugins.test.ts", "const m = await import(name);\n"));
    let dir = repo("unknown-root", &files);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/docs/src/site.ts")],
    );
    assert!(tests(&plan).contains(&"tests/plugins.test.ts"));
    assert!(!plan.all);
}

#[test]
fn an_unresolved_local_import_depends_on_its_whole_module() {
    let mut files = WORKSPACE.to_vec();
    files.push((
        "packages/billing/test/alias.test.ts",
        "import x from '#internal/x';\n",
    ));
    let dir = repo("unresolved-local", &files);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/billing/src/invoice.ts")],
    );
    assert!(tests(&plan).contains(&"packages/billing/test/alias.test.ts"));
    assert!(plan.warnings.iter().any(|w| w.code == "unresolved-import"));
}

#[test]
fn a_file_whose_tsconfig_couldnt_be_applied_depends_on_its_whole_module() {
    let mut files = WORKSPACE.to_vec();
    files.push((
        "packages/billing/tsconfig.json",
        r#"{ "extends": "@missing/config" }"#,
    ));
    files.push((
        "packages/billing/test/fallback.test.ts",
        "import { x } from './sibling';\n",
    ));
    files.push(("packages/billing/test/sibling.ts", "export const x = 1;\n"));
    let dir = repo("fallback", &files);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/billing/src/invoice.ts")],
    );
    assert!(tests(&plan).contains(&"packages/billing/test/fallback.test.ts"));
}

#[test]
fn unresolved_imports_fail_the_plan_when_configured_to() {
    let mut files = WORKSPACE.to_vec();
    files.push((
        "packages/core/src/broken.ts",
        "import x from './missing';\n",
    ));
    let dir = repo("unresolved-fail", &files);
    let cfg = config(&format!("{VITEST}\n[graph]\nunresolved = \"fail\"\n"));
    let err = try_plan(&dir, &cfg, vec![modified("packages/core/src/money.ts")]).unwrap_err();
    assert!(err.contains("./missing"), "{err}");
}

#[test]
fn a_test_no_runner_matches_fails_the_plan_naming_it() {
    let dir = repo("no-runner", WORKSPACE);
    let cfg = config("[[tests.runners]]\nid = \"vitest\"\nmatch = [\"packages/core/**\"]\ncommand = [\"vitest\"]\n");
    let err = try_plan(&dir, &cfg, vec![modified("packages/core/src/money.ts")]).unwrap_err();
    assert!(
        err.contains("packages/billing/test/invoice.test.ts"),
        "{err}"
    );
}

#[test]
fn a_runner_leaves_its_excluded_files_to_another() {
    let dir = repo("runner-exclude", WORKSPACE);
    let cfg = config(
        r#"
[[tests.runners]]
id = "vitest"
match = ["packages/**"]
exclude = ["packages/billing/**"]
command = ["vitest", "run", "{files}"]

[[tests.runners]]
id = "jest"
match = ["packages/billing/**"]
command = ["jest", "{files}"]
"#,
    );
    let plan = run(&dir, &cfg, vec![modified("packages/core/src/money.ts")]);
    let jest = plan.invocations.iter().find(|i| i.id == "jest").unwrap();
    assert!(jest
        .argv
        .contains(&"packages/billing/test/invoice.test.ts".to_string()));
    let vitest = plan.invocations.iter().find(|i| i.id == "vitest").unwrap();
    assert!(vitest
        .argv
        .iter()
        .all(|a| !a.starts_with("packages/billing/")));
}

#[test]
fn an_ignored_location_never_swallows_a_source_file() {
    let mut files = WORKSPACE.to_vec();
    files.push(("docs/site.config.ts", "export default {};\n"));
    let dir = repo("ignore-source", &files);
    let plan = run(&dir, &config(VITEST), vec![modified("docs/site.config.ts")]);
    assert!(plan.ignored.is_empty());
    assert_eq!(plan.unreached[0].path, "docs/site.config.ts");
    assert!(plan.all, "an unreached root file widens to everything");
}

#[test]
fn deleting_a_test_or_changing_a_manifest_is_not_reported_unreached() {
    let dir = repo("not-unreached", WORKSPACE);
    let gone = run(
        &dir,
        &config(VITEST),
        vec![deleted("packages/core/test/gone.test.ts")],
    );
    assert!(
        gone.unreached.is_empty() && !gone.all,
        "{:?}",
        gone.unreached
    );
    let manifest = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/docs/package.json")],
    );
    assert!(manifest.unreached.is_empty(), "{:?}", manifest.unreached);
}

#[test]
fn a_manifest_whose_package_has_no_files_is_still_unreached() {
    let mut files = WORKSPACE.to_vec();
    files.push((
        "packages/prebuilt/package.json",
        r#"{ "name": "prebuilt" }"#,
    ));
    let dir = repo("manifest-only", &files);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![modified("packages/prebuilt/package.json")],
    );
    assert_eq!(plan.unreached[0].path, "packages/prebuilt/package.json");
    assert!(plan.all);
}
