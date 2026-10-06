//! Changes that used to select everything but can't change what a test
//! does: dependency versions moving, Fairlead's own config, and a workflow
//! run only by hand or on a schedule.

mod common;

use std::collections::BTreeMap;

use common::*;
use fairlead_core::plan::{Change, Plan, Reason, Status};

const BUN: &str = r#"
[[tests.runners]]
id = "bun"
match = ["**/*.test.ts"]
command = ["bun", "test", "{files}"]
"#;

const MANIFEST: &str = r#"{
  "name": "app",
  "private": true,
  "dependencies": { "left": "1.0.0", "outer": "1.0.0" },
  "devDependencies": { "tool": "1.0.0" }
}"#;

const LOCK: &str = r#"{
  "lockfileVersion": 1,
  "workspaces": {
    "": {
      "name": "app",
      "dependencies": { "left": "1.0.0", "outer": "1.0.0", },
      "devDependencies": { "tool": "1.0.0", },
    },
  },
  "packages": {
    "left": ["left@1.0.0", "", {}, "sha512-left1"],
    "outer": ["outer@1.0.0", "", { "dependencies": { "inner": "^1.0.0" } }, "sha512-outer1"],
    "inner": ["inner@1.0.0", "", {}, "sha512-inner1"],
    "tool": ["tool@1.0.0", "", { "bin": { "tool-cli": "bin/cli.js" } }, "sha512-tool1"],
  }
}
"#;

fn files<'a>(manifest: &'a str, lock: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("package.json", manifest),
        ("bun.lock", lock),
        (
            "src/pad.ts",
            "import left from 'left';\nexport const pad = left;\n",
        ),
        (
            "src/wrap.ts",
            "import outer from 'outer';\nexport const wrap = outer;\n",
        ),
        ("src/plain.ts", "export const plain = 1;\n"),
        ("test/pad.test.ts", "import { pad } from '../src/pad';\n"),
        ("test/wrap.test.ts", "import { wrap } from '../src/wrap';\n"),
        (
            "test/plain.test.ts",
            "import { plain } from '../src/plain';\n",
        ),
    ]
}

/// `name`'s version moved from 1.0.0 to 1.0.1 in the manifest and the lock.
fn bump(text: &str, name: &str) -> String {
    text.replace(
        &format!("\"{name}\": \"1.0.0\""),
        &format!("\"{name}\": \"1.0.1\""),
    )
    .replace(&format!("{name}@1.0.0"), &format!("{name}@1.0.1"))
    .replace(&format!("sha512-{name}1"), &format!("sha512-{name}2"))
}

fn base(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(p, t)| (p.to_string(), t.to_string()))
        .collect()
}

fn codes(plan: &Plan) -> Vec<&str> {
    plan.warnings.iter().map(|w| w.code.as_str()).collect()
}

/// Plans a bump of `name` in both files, against the fixture as the base.
fn plan_bump(dir_name: &str, name: &str, toml: &str) -> Plan {
    let (manifest, lock) = (bump(MANIFEST, name), bump(LOCK, name));
    let dir = repo(dir_name, &files(&manifest, &lock));
    let changes = vec![modified("bun.lock"), modified("package.json")];
    let at_base = base(&[("package.json", MANIFEST), ("bun.lock", LOCK)]);
    try_plan_with_base(&dir, &config(toml), changes, at_base).unwrap()
}

#[test]
fn a_bumped_dev_dependency_nothing_imports_selects_nothing() {
    let plan = plan_bump("bump-tool", "tool", BUN);
    assert!(!plan.all);
    assert!(tests(&plan).is_empty(), "{:?}", tests(&plan));
    assert!(plan.unreached.is_empty());
    assert_eq!(codes(&plan), ["version-bump-scoped", "version-bump-scoped"]);
    assert!(plan.warnings[1].message.contains("`tool`"));
}

#[test]
fn a_bumped_package_selects_the_tests_reaching_its_importers() {
    let plan = plan_bump("bump-left", "left", BUN);
    assert!(!plan.all);
    assert_eq!(tests(&plan), ["test/pad.test.ts"]);
    assert_eq!(
        reason(&plan, "test/pad.test.ts"),
        &Reason::Import {
            chain: vec![
                "package.json (left)".into(),
                "src/pad.ts".into(),
                "test/pad.test.ts".into()
            ]
        }
    );
}

#[test]
fn an_installed_package_is_found_through_node_modules() {
    let (manifest, lock) = (bump(MANIFEST, "left"), bump(LOCK, "left"));
    let mut fixture = files(&manifest, &lock);
    fixture.push((
        "node_modules/left/package.json",
        r#"{ "name": "left", "main": "index.js" }"#,
    ));
    fixture.push(("node_modules/left/index.js", "module.exports = 1;\n"));
    let dir = repo("bump-installed", &fixture);
    let scan = fairlead_lang::build(&dir, &config(BUN)).unwrap();
    let pad = scan.graph.id("src/pad.ts").unwrap();
    assert!(scan.graph.external.contains(&(pad, "left".to_string())));
    assert!(scan.graph.failed.iter().all(|(from, _, _)| *from != pad));
    let changes = vec![modified("bun.lock"), modified("package.json")];
    let at_base = base(&[("package.json", MANIFEST), ("bun.lock", LOCK)]);
    let plan = try_plan_with_base(&dir, &config(BUN), changes, at_base).unwrap();
    assert_eq!(tests(&plan), ["test/pad.test.ts"]);
}

#[test]
fn a_package_depending_on_a_bumped_one_carries_its_importers() {
    let lock = bump(LOCK, "inner");
    let dir = repo("bump-inner", &files(MANIFEST, &lock));
    let changes = vec![modified("bun.lock")];
    let plan =
        try_plan_with_base(&dir, &config(BUN), changes, base(&[("bun.lock", LOCK)])).unwrap();
    assert!(!plan.all);
    assert_eq!(tests(&plan), ["test/wrap.test.ts"]);
    assert_eq!(
        reason(&plan, "test/wrap.test.ts"),
        &Reason::Import {
            chain: vec![
                "bun.lock (outer)".into(),
                "src/wrap.ts".into(),
                "test/wrap.test.ts".into()
            ]
        }
    );
}

#[test]
fn a_manifest_change_beyond_versions_selects_everything() {
    let manifest = bump(MANIFEST, "tool").replace("\"private\": true", "\"private\": false");
    let dir = repo("bump-more", &files(&manifest, LOCK));
    let changes = vec![modified("package.json")];
    let plan = try_plan_with_base(
        &dir,
        &config(BUN),
        changes,
        base(&[("package.json", MANIFEST)]),
    )
    .unwrap();
    assert!(plan.all);
    assert_eq!(
        reason(&plan, "test/plain.test.ts"),
        &Reason::RunAll {
            path: "package.json".into()
        }
    );
}

#[test]
fn a_lock_that_adds_a_package_selects_everything() {
    let lock = bump(LOCK, "tool").replace(
        r#""inner": ["inner@1.0.0", "", {}, "sha512-inner1"],"#,
        r#""inner": ["inner@1.0.0", "", {}, "sha512-inner1"], "extra": ["extra@1.0.0", "", {}, "sha512-x"],"#,
    );
    let dir = repo("bump-added", &files(&bump(MANIFEST, "tool"), &lock));
    let changes = vec![modified("bun.lock"), modified("package.json")];
    let at_base = base(&[("package.json", MANIFEST), ("bun.lock", LOCK)]);
    let plan = try_plan_with_base(&dir, &config(BUN), changes, at_base).unwrap();
    assert!(plan.all);
    assert_eq!(
        reason(&plan, "test/plain.test.ts"),
        &Reason::RunAll {
            path: "bun.lock".into()
        }
    );
}

#[test]
fn a_bumped_package_a_runner_runs_selects_everything() {
    let toml = BUN.replace(
        r#"["bun", "test", "{files}"]"#,
        r#"["npx", "tool-cli", "{files}"]"#,
    );
    let plan = plan_bump("bump-runner", "tool", &toml);
    assert!(plan.all);
    assert_eq!(codes(&plan), ["version-bump-runs-everything"]);
    assert!(plan.warnings[0].message.contains("runner `bun`"));
}

#[test]
fn a_bumped_package_a_run_all_file_imports_selects_everything() {
    let (manifest, lock) = (bump(MANIFEST, "tool"), bump(LOCK, "tool"));
    let mut fixture = files(&manifest, &lock);
    fixture.push((
        "vitest.config.ts",
        "import tool from 'tool';\nexport default tool;\n",
    ));
    let dir = repo("bump-config-import", &fixture);
    let changes = vec![modified("bun.lock"), modified("package.json")];
    let at_base = base(&[("package.json", MANIFEST), ("bun.lock", LOCK)]);
    let plan = try_plan_with_base(&dir, &config(BUN), changes, at_base).unwrap();
    assert!(plan.all);
    assert_eq!(codes(&plan), ["version-bump-runs-everything"]);
}

#[test]
fn a_bump_without_its_base_text_or_a_bun_lock_selects_everything() {
    let manifest = bump(MANIFEST, "tool");
    let dir = repo("bump-no-base", &files(&manifest, LOCK));
    let plan = try_plan(&dir, &config(BUN), vec![modified("package.json")]).unwrap();
    assert!(plan.all);
    let mut fixture = files(&manifest, LOCK);
    fixture.retain(|(p, _)| *p != "bun.lock");
    let dir = repo("bump-no-lock", &fixture);
    let at_base = base(&[("package.json", MANIFEST)]);
    let plan =
        try_plan_with_base(&dir, &config(BUN), vec![modified("package.json")], at_base).unwrap();
    assert!(plan.all);
}

#[test]
fn a_workspace_manifest_bump_selects_importers_not_its_whole_package() {
    let member = r#"{ "name": "member", "dependencies": { "left": "1.0.0" } }"#;
    let bumped = member.replace("1.0.0", "1.0.1");
    let root = r#"{ "name": "app", "private": true, "workspaces": ["packages/*"] }"#;
    let dir = repo(
        "bump-member",
        &[
            ("package.json", root),
            ("bun.lock", LOCK),
            ("packages/member/package.json", &bumped),
            (
                "packages/member/src/pad.ts",
                "import left from 'left';\nexport const pad = left;\n",
            ),
            ("packages/member/src/plain.ts", "export const plain = 1;\n"),
            (
                "packages/member/test/pad.test.ts",
                "import { pad } from '../src/pad';\n",
            ),
            (
                "packages/member/test/plain.test.ts",
                "import { plain } from '../src/plain';\n",
            ),
        ],
    );
    let changes = vec![modified("packages/member/package.json")];
    let at_base = base(&[("packages/member/package.json", member)]);
    let plan = try_plan_with_base(&dir, &config(BUN), changes, at_base).unwrap();
    assert_eq!(tests(&plan), ["packages/member/test/pad.test.ts"]);
}

#[test]
fn a_config_change_selects_nothing_by_itself() {
    let dir = repo("config", &files(MANIFEST, LOCK));
    for path in ["fairlead.toml", "fairlead.ci.yaml"] {
        let changes = vec![modified(path), modified("src/plain.ts")];
        let plan = try_plan(&dir, &config(BUN), changes).unwrap();
        assert!(!plan.all, "{path}");
        assert_eq!(tests(&plan), ["test/plain.test.ts"]);
        assert!(plan.unreached.is_empty());
        assert_eq!(codes(&plan), ["fairlead-config"]);
    }
}

#[test]
fn a_config_file_in_run_all_still_selects_everything() {
    let dir = repo("config-run-all", &files(MANIFEST, LOCK));
    let toml = format!("{BUN}\n[plan]\nrun_all = [\"fairlead.toml\"]\n");
    let plan = try_plan(&dir, &config(&toml), vec![modified("fairlead.toml")]).unwrap();
    assert!(plan.all);
}

const NIGHTLY: &str = "name: nightly\non:\n  workflow_dispatch:\n  schedule:\n    - cron: '0 3 * * *'\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";
const WORKFLOW: &str = ".github/workflows/nightly.yml";

fn workflow_plan(name: &str, head: Option<&str>, at_base: Option<&str>, status: Status) -> Plan {
    let mut fixture = files(MANIFEST, LOCK);
    if let Some(text) = head {
        fixture.push((WORKFLOW, text));
    }
    let dir = repo(name, &fixture);
    let change = Change {
        path: WORKFLOW.into(),
        status,
        from: None,
    };
    let base_files = at_base.map(|t| base(&[(WORKFLOW, t)])).unwrap_or_default();
    try_plan_with_base(&dir, &config(BUN), vec![change], base_files).unwrap()
}

#[test]
fn a_dispatch_only_workflow_selects_nothing() {
    let head = NIGHTLY.replace("echo hi", "echo hello");
    let plan = workflow_plan("wf-modified", Some(&head), Some(NIGHTLY), Status::Modified);
    assert!(!plan.all);
    assert!(tests(&plan).is_empty());
    assert_eq!(codes(&plan), ["workflow-dispatch-only"]);
    let plan = workflow_plan("wf-added", Some(NIGHTLY), None, Status::Added);
    assert!(!plan.all);
    let plan = workflow_plan("wf-deleted", None, Some(NIGHTLY), Status::Deleted);
    assert!(!plan.all);
}

#[test]
fn a_workflow_with_another_trigger_at_either_side_selects_everything() {
    let push = NIGHTLY.replace("  workflow_dispatch:\n", "  workflow_dispatch:\n  push:\n");
    let plan = workflow_plan(
        "wf-gains-push",
        Some(&push),
        Some(NIGHTLY),
        Status::Modified,
    );
    assert!(plan.all);
    let plan = workflow_plan(
        "wf-loses-push",
        Some(NIGHTLY),
        Some(&push),
        Status::Modified,
    );
    assert!(plan.all);
    let plan = workflow_plan("wf-deleted-push", None, Some(&push), Status::Deleted);
    assert!(plan.all);
    let plan = workflow_plan("wf-no-base", Some(NIGHTLY), None, Status::Modified);
    assert!(plan.all);
    let plan = workflow_plan(
        "wf-unparsable",
        Some("on: [workflow_dispatch"),
        Some(NIGHTLY),
        Status::Modified,
    );
    assert!(plan.all);
}
