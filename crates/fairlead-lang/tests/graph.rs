//! Each resolver case on a synthetic repository built by the test.

use std::fs;
use std::path::{Path, PathBuf};

use fairlead_core::config::Config;
use fairlead_lang::{build, EdgeKind, Scan};

fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-graph-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).unwrap();
    for (path, text) in files {
        let full = dir.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }
    dir
}

fn scan(dir: &Path) -> Scan {
    build(dir, &Config::default()).unwrap()
}

/// The files `file` depends on directly, with the edge kind.
fn deps(scan: &Scan, file: &str) -> Vec<(String, EdgeKind)> {
    let id = scan
        .graph
        .id(file)
        .unwrap_or_else(|| panic!("{file} not in the tree"));
    let mut out: Vec<(String, EdgeKind)> = scan
        .graph
        .dependencies(id)
        .iter()
        .map(|&(to, kind)| (scan.graph.files[to as usize].clone(), kind))
        .collect();
    out.sort();
    out
}

fn depends(scan: &Scan, from: &str, to: &str) -> bool {
    let (a, b) = (scan.graph.id(from).unwrap(), scan.graph.id(to).unwrap());
    scan.graph.affected(&[b]).contains_key(&a)
}

#[test]
fn relative_js_specifiers_reach_ts_files_and_directories_their_index() {
    let dir = repo(
        "relative",
        &[
            (
                "src/a.ts",
                "import { b } from './b.js';\nimport { c } from './c';\n",
            ),
            ("src/b.ts", "export const b = 1;\n"),
            ("src/c/index.ts", "export const c = 1;\n"),
        ],
    );
    let s = scan(&dir);
    assert_eq!(
        deps(&s, "src/a.ts"),
        vec![
            ("src/b.ts".into(), EdgeKind::Import),
            ("src/c/index.ts".into(), EdgeKind::Import)
        ]
    );
}

#[test]
fn tsconfig_paths_aliases_resolve() {
    let dir = repo(
        "paths",
        &[
            (
                "tsconfig.json",
                r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["src/*"] } } }"#,
            ),
            ("src/a.ts", "import { b } from '@/lib/b';\n"),
            ("src/lib/b.ts", "export const b = 1;\n"),
        ],
    );
    let s = scan(&dir);
    assert_eq!(
        deps(&s, "src/a.ts"),
        vec![("src/lib/b.ts".into(), EdgeKind::Import)]
    );
    assert!(s.graph.unresolved.is_empty(), "{:?}", s.graph.unresolved);
}

#[test]
fn workspace_packages_resolve_through_exports_without_an_install() {
    let dir = repo("workspace", &[
        ("package.json", r#"{ "private": true, "workspaces": ["packages/*"] }"#),
        ("packages/core/package.json", r#"{ "name": "@acme/core", "exports": { ".": "./src/index.ts", "./util": "./src/util.ts" } }"#),
        ("packages/core/src/index.ts", "export const core = 1;\n"),
        ("packages/core/src/util.ts", "export const util = 1;\n"),
        ("packages/app/package.json", r#"{ "name": "app" }"#),
        ("packages/app/src/main.ts", "import { core } from '@acme/core';\nimport { util } from '@acme/core/util';\nimport fs from 'node:fs';\nimport x from 'left-pad';\n"),
    ]);
    let s = scan(&dir);
    assert!(!dir.join("node_modules").exists());
    assert_eq!(
        deps(&s, "packages/app/src/main.ts"),
        vec![
            ("packages/core/src/index.ts".into(), EdgeKind::Import),
            ("packages/core/src/util.ts".into(), EdgeKind::Import)
        ]
    );
    assert!(
        s.graph.unresolved.is_empty(),
        "externals aren't unresolved: {:?}",
        s.graph.unresolved
    );
}

#[test]
fn export_conditions_come_from_config() {
    let files = [
        ("pnpm-workspace.yaml", "packages:\n  - 'libs/*'\n"),
        (
            "libs/a/package.json",
            r#"{ "name": "a", "exports": { ".": { "source": "./src/index.ts", "default": "./dist/index.js" } } }"#,
        ),
        ("libs/a/src/index.ts", "export const a = 1;\n"),
        ("libs/b/package.json", r#"{ "name": "b" }"#),
        ("libs/b/main.ts", "import { a } from 'a';\n"),
    ];
    let dir = repo("conditions", &files);
    let mut config = Config::default();
    config.graph.conditions = vec![
        "source".to_string(),
        "import".to_string(),
        "default".to_string(),
    ]
    .into();
    let s = build(&dir, &config).unwrap();
    assert_eq!(
        deps(&s, "libs/b/main.ts"),
        vec![("libs/a/src/index.ts".into(), EdgeKind::Import)]
    );
}

#[test]
fn build_output_maps_back_to_source_through_the_package_tsconfig() {
    let dir = repo("outdir", &[
        ("package.json", r#"{ "workspaces": ["packages/*"] }"#),
        ("packages/lib/package.json", r#"{ "name": "lib", "main": "lib/index.js", "exports": { ".": "./lib/index.js" } }"#),
        ("packages/lib/tsconfig.json", "{\n  // built into lib/\n  \"compilerOptions\": { \"outDir\": \"lib\", \"rootDir\": \"src\", },\n}\n"),
        ("packages/lib/src/index.ts", "export const lib = 1;\n"),
        ("packages/app/package.json", r#"{ "name": "app" }"#),
        ("packages/app/main.ts", "import { lib } from 'lib';\n"),
    ]);
    let s = scan(&dir);
    assert_eq!(
        deps(&s, "packages/app/main.ts"),
        vec![("packages/lib/src/index.ts".into(), EdgeKind::Import)]
    );
}

#[test]
fn a_workspace_import_with_no_file_on_disk_depends_on_the_whole_package() {
    let dir = repo(
        "package-edge",
        &[
            ("package.json", r#"{ "workspaces": ["packages/*"] }"#),
            (
                "packages/bundled/package.json",
                r#"{ "name": "bundled", "exports": { ".": "./dist/index.js" } }"#,
            ),
            (
                "packages/bundled/src/deep/thing.ts",
                "export const t = 1;\n",
            ),
            ("packages/app/package.json", r#"{ "name": "app" }"#),
            ("packages/app/main.ts", "import { t } from 'bundled';\n"),
        ],
    );
    let s = scan(&dir);
    assert!(depends(
        &s,
        "packages/app/main.ts",
        "packages/bundled/src/deep/thing.ts"
    ));
    assert_eq!(s.graph.stats().package_edges, 1);
}

#[test]
fn a_tsconfig_that_extends_something_unreadable_still_resolves_relative_imports() {
    let dir = repo(
        "extends",
        &[
            (
                "tsconfig.json",
                r#"{ "extends": "@missing/shared-config", "compilerOptions": {} }"#,
            ),
            ("src/a.ts", "import { b } from './b';\n"),
            ("src/b.ts", "export const b = 1;\n"),
        ],
    );
    let s = scan(&dir);
    assert_eq!(
        deps(&s, "src/a.ts"),
        vec![("src/b.ts".into(), EdgeKind::Import)]
    );
}

#[test]
fn type_imports_are_edges_unless_config_turns_them_off() {
    let files = [
        ("a.ts", "import type { B } from './b';\n"),
        ("b.ts", "export type B = 1;\n"),
    ];
    let dir = repo("types", &files);
    assert_eq!(
        deps(&scan(&dir), "a.ts"),
        vec![("b.ts".into(), EdgeKind::TypeImport)]
    );
    let mut config = Config::default();
    config.graph.type_imports = false;
    assert!(deps(&build(&dir, &config).unwrap(), "a.ts").is_empty());
}

#[test]
fn tests_reach_files_they_name_mock_or_snapshot() {
    let dir = repo(
        "literals",
        &[
            ("cli/bin/run.mjs", "console.log('run');\n"),
            (
                "cli/test/helpers.ts",
                "export const bin = '../bin/run.mjs';\n",
            ),
            (
                "cli/test/run.test.ts",
                "import { bin } from './helpers';\nvi.mock('../src/dep');\n",
            ),
            ("cli/src/dep.ts", "export const dep = 1;\n"),
            (
                "cli/test/__snapshots__/run.test.ts.snap",
                "exports[`x`] = `1`;\n",
            ),
        ],
    );
    let s = scan(&dir);
    assert!(
        depends(&s, "cli/test/run.test.ts", "cli/bin/run.mjs"),
        "through the helper's path literal"
    );
    assert!(
        depends(&s, "cli/test/run.test.ts", "cli/src/dep.ts"),
        "through the mock"
    );
    assert!(
        depends(
            &s,
            "cli/test/run.test.ts",
            "cli/test/__snapshots__/run.test.ts.snap"
        ),
        "through the snapshot"
    );
    let chain = s
        .graph
        .why(
            s.graph.id("cli/test/run.test.ts").unwrap(),
            s.graph.id("cli/bin/run.mjs").unwrap(),
        )
        .unwrap();
    let files: Vec<&str> = chain.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(
        files,
        [
            "cli/test/run.test.ts",
            "cli/test/helpers.ts",
            "cli/bin/run.mjs"
        ]
    );
}

#[test]
fn unknown_dynamic_imports_and_unresolved_specifiers_are_recorded() {
    let dir = repo(
        "unknowns",
        &[(
            "a.ts",
            "const m = await import(name);\nimport x from './missing';\n",
        )],
    );
    let s = scan(&dir);
    assert_eq!(s.graph.unknown.len(), 1);
    assert_eq!(
        s.graph
            .unresolved
            .iter()
            .map(|(_, spec)| spec.as_str())
            .collect::<Vec<_>>(),
        ["./missing"]
    );
}

#[test]
fn workspace_globs_keep_star_within_one_segment_and_honour_negation() {
    let dir = repo(
        "globs",
        &[
            (
                "pnpm-workspace.yaml",
                "packages:\n  - 'pkgs/*'\n  - '!pkgs/skip'\n",
            ),
            ("pkgs/one/package.json", r#"{ "name": "one" }"#),
            ("pkgs/skip/package.json", r#"{ "name": "skip" }"#),
            (
                "pkgs/one/fixtures/nested/package.json",
                r#"{ "name": "fixture" }"#,
            ),
        ],
    );
    let s = scan(&dir);
    assert_eq!(s.graph.packages, vec!["one".to_string()]);
}
