//! A changed pnpm lockfile selects the workspace packages whose resolved
//! dependencies changed, their dependents, and checks watching manifests.

mod common;

use std::collections::BTreeMap;

use common::*;
use fairlead_core::plan::{Plan, Reason};

const FILES: &[(&str, &str)] = &[
    (
        "package.json",
        r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    ),
    (
        "packages/core/package.json",
        r#"{ "name": "core", "exports": { ".": "./src/index.ts" } }"#,
    ),
    ("packages/core/src/index.ts", "export const money = 1;\n"),
    (
        "packages/core/test/money.test.ts",
        "import { money } from '../src/index';\n",
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
    ("packages/docs/package.json", r#"{ "name": "docs" }"#),
    ("packages/docs/src/site.ts", "export const site = 1;\n"),
    (
        "packages/docs/test/site.test.ts",
        "import { site } from '../src/site';\n",
    ),
];

const LOCK: &str = r#"lockfileVersion: '9.0'
settings:
  autoInstallPeers: true
importers:
  .:
    devDependencies:
      tool:
        specifier: ^1.0.0
        version: 1.0.0
  packages/core:
    dependencies:
      left:
        specifier: ^1.0.0
        version: 1.0.0
  packages/billing:
    dependencies:
      core:
        specifier: workspace:*
        version: link:../core
  packages/docs:
    dependencies: {}
packages:
  tool@1.0.0:
    resolution: {integrity: sha512-tool}
  left@1.0.0:
    resolution: {integrity: sha512-left}
snapshots:
  tool@1.0.0: {}
  left@1.0.0: {}
"#;

const CONFIG: &str = r#"
[[tests.runners]]
id = "vitest"
match = ["**"]
command = ["vitest", "run", "{files}"]

[plan]
lockfile = "scope"

[[checks]]
id = "typecheck"
command = ["tsc"]
paths = ["packages/*/package.json"]
"#;

fn repo_with(name: &str, head_lock: &str, extra: &[(&str, &str)]) -> std::path::PathBuf {
    let mut files = FILES.to_vec();
    files.push(("pnpm-lock.yaml", head_lock));
    files.extend_from_slice(extra);
    repo(name, &files)
}

fn plan_lock(dir: &std::path::Path, toml_text: &str, base_lock: Option<&str>) -> Plan {
    let base: BTreeMap<String, String> = base_lock
        .map(|b| [("pnpm-lock.yaml".to_string(), b.to_string())].into())
        .unwrap_or_default();
    try_plan_with_base(
        dir,
        &config(toml_text),
        vec![modified("pnpm-lock.yaml")],
        base,
    )
    .unwrap()
}

#[test]
fn a_dependency_bump_selects_its_package_and_dependents_only() {
    let head = LOCK.replace("sha512-left", "sha512-left-2");
    let dir = repo_with("lock-scope", &head, &[]);
    let plan = plan_lock(&dir, CONFIG, Some(LOCK));
    assert!(!plan.all);
    assert_eq!(
        tests(&plan),
        [
            "packages/billing/test/invoice.test.ts",
            "packages/core/test/money.test.ts"
        ]
    );
    assert!(
        plan.unreached.is_empty(),
        "a scoped lockfile isn't unreached: {:?}",
        plan.unreached
    );
    assert!(
        plan.checks.iter().any(|c| c.id == "typecheck"),
        "a check watching manifests sees the package the lockfile reached"
    );
    let why = format!("{:?}", reason(&plan, "packages/core/test/money.test.ts"));
    assert!(why.contains("pnpm-lock.yaml (packages/core)"), "{why}");
}

#[test]
fn a_root_dependency_bump_selects_everything() {
    let head = LOCK.replace("sha512-tool", "sha512-tool-2");
    let dir = repo_with("lock-root", &head, &[]);
    let plan = plan_lock(&dir, CONFIG, Some(LOCK));
    assert!(plan.all);
    assert!(
        matches!(reason(&plan, "packages/docs/test/site.test.ts"), Reason::RunAll { path } if path == "pnpm-lock.yaml")
    );
}

#[test]
fn without_the_base_text_scoping_off_or_hoisting_the_lockfile_selects_everything() {
    let head = LOCK.replace("sha512-left", "sha512-left-2");
    let dir = repo_with("lock-nobase", &head, &[]);
    assert!(plan_lock(&dir, CONFIG, None).all, "no base text");
    let off = CONFIG.replace("lockfile = \"scope\"", "lockfile = \"all\"");
    assert!(plan_lock(&dir, &off, Some(LOCK)).all, "plan.lockfile = all");
    let default = CONFIG.replace("lockfile = \"scope\"", "");
    assert!(plan_lock(&dir, &default, Some(LOCK)).all, "off by default");
    let hoisted = repo_with(
        "lock-hoisted",
        &head,
        &[(".npmrc", "node-linker = hoisted\n")],
    );
    assert!(
        plan_lock(&hoisted, CONFIG, Some(LOCK)).all,
        "hoisted node_modules"
    );
    let unreadable = repo_with(
        "lock-override",
        &head.replace("settings:", "overrides:\n  left: 1.0.1\nsettings:"),
        &[],
    );
    assert!(
        plan_lock(&unreadable, CONFIG, Some(LOCK)).all,
        "an overrides change"
    );
}

#[test]
fn a_lockfile_change_that_reaches_no_package_says_so() {
    let base = LOCK.replace("settings:", "catalogs:\n  default:\n    left:\n      specifier: ^1.0.0\n      version: 1.0.0\nsettings:");
    let head = base.replace(
        "specifier: ^1.0.0\n      version: 1.0.0\nsettings",
        "specifier: ^1.0.1\n      version: 1.0.0\nsettings",
    );
    let dir = repo_with("lock-nothing", &head, &[]);
    let plan = plan_lock(&dir, CONFIG, Some(&base));
    assert!(plan.tests.is_empty() && !plan.all);
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.code == "lockfile-scoped-to-nothing"));
}
