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
fn ignored_build_output_on_disk_still_maps_to_source_or_package() {
    let dir = repo(
        "built",
        &[
            ("package.json", r#"{ "workspaces": ["packages/*"] }"#),
            (".gitignore", "/packages/*/lib/\ndist/\n"),
            (
                "packages/lib/package.json",
                r#"{ "name": "lib", "exports": { ".": "./lib/index.js" } }"#,
            ),
            (
                "packages/lib/tsconfig.json",
                r#"{ "compilerOptions": { "outDir": "lib", "rootDir": "src" } }"#,
            ),
            ("packages/lib/src/index.ts", "export const lib = 1;\n"),
            ("packages/lib/lib/index.js", "exports.lib = 1;\n"),
            (
                "packages/bundled/package.json",
                r#"{ "name": "bundled", "exports": { ".": "./dist/index.js" } }"#,
            ),
            ("packages/bundled/src/thing.ts", "export const t = 1;\n"),
            ("packages/bundled/dist/index.js", "exports.t = 1;\n"),
            ("packages/app/package.json", r#"{ "name": "app" }"#),
            (
                "packages/app/main.ts",
                "import { lib } from 'lib';\nimport { t } from 'bundled';\n",
            ),
        ],
    );
    let s = scan(&dir);
    assert_eq!(
        deps(&s, "packages/app/main.ts"),
        vec![("packages/lib/src/index.ts".into(), EdgeKind::Import)]
    );
    assert!(depends(
        &s,
        "packages/app/main.ts",
        "packages/bundled/src/thing.ts"
    ));
    assert!(s.graph.unresolved.is_empty(), "{:?}", s.graph.unresolved);
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

#[test]
fn a_second_build_reuses_every_parse_and_reparses_only_what_changed() {
    let dir = repo(
        "cache",
        &[
            ("src/a.ts", "import { b } from './b';\n"),
            ("src/b.ts", "export const b = 1;\n"),
            ("src/c.ts", "export const c = 1;\n"),
        ],
    );
    let first = scan(&dir);
    assert_eq!((first.cache.hits, first.cache.misses), (0, 3));
    assert!(dir.join(".git/fairlead/parse-cache.json").is_file());
    let second = scan(&dir);
    assert_eq!((second.cache.hits, second.cache.misses), (3, 0));
    assert_eq!(first.graph.stats(), second.graph.stats());

    fs::write(dir.join("src/a.ts"), "import { c } from './c';\n").unwrap();
    let third = scan(&dir);
    assert_eq!((third.cache.hits, third.cache.misses), (2, 1));
    assert_eq!(
        deps(&third, "src/a.ts"),
        vec![("src/c.ts".into(), EdgeKind::Import)]
    );
}

#[test]
fn an_unreadable_cache_is_rebuilt_and_a_disabled_one_is_never_written() {
    let dir = repo("cache-bad", &[("a.ts", "import './b';\n"), ("b.ts", "")]);
    fs::create_dir_all(dir.join(".git/fairlead")).unwrap();
    fs::write(dir.join(".git/fairlead/parse-cache.json"), "{ not json").unwrap();
    let s = scan(&dir);
    assert_eq!((s.cache.hits, s.cache.misses), (0, 2));
    assert_eq!(deps(&s, "a.ts"), vec![("b.ts".into(), EdgeKind::Import)]);
    assert_eq!(scan(&dir).cache.hits, 2, "rewritten after the bad read");

    let path = dir.join(".git/fairlead/parse-cache.json");
    let mut stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    stored["version"] = "an older build".into();
    fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    assert_eq!(
        scan(&dir).cache.hits,
        0,
        "another version's entries aren't used"
    );

    let off = repo("cache-off", &[("a.ts", "")]);
    let mut config = Config::default();
    config.graph.cache = false;
    let s = build(&off, &config).unwrap();
    assert!(!s.cache.enabled);
    assert!(!off.join(".git/fairlead").exists());
}

#[test]
fn a_worktree_keeps_its_cache_in_the_git_dir_its_git_file_names() {
    let dir = repo("cache-worktree", &[("a.ts", "")]);
    fs::remove_dir_all(dir.join(".git")).unwrap();
    let git_dir = dir.with_file_name(format!(
        "{}-gitdir",
        dir.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_dir_all(&git_dir);
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(dir.join(".git"), format!("gitdir: {}\n", git_dir.display())).unwrap();
    let s = scan(&dir);
    assert!(s.cache.enabled);
    assert!(git_dir.join("fairlead/parse-cache.json").is_file());
}

#[test]
fn a_solution_tsconfig_with_references_resolves_each_project_by_its_own_paths() {
    let dir = repo(
        "references",
        &[
            (
                "tsconfig.json",
                r#"{ "files": [], "references": [{ "path": "./packages/app" }, { "path": "./packages/lib" }] }"#,
            ),
            (
                "packages/app/tsconfig.json",
                r##"{ "compilerOptions": { "composite": true, "baseUrl": ".", "paths": { "#lib/*": ["../lib/src/*"] } }, "references": [{ "path": "../lib" }] }"##,
            ),
            ("packages/app/src/main.ts", "import { x } from '#lib/x';\n"),
            (
                "packages/lib/tsconfig.json",
                r#"{ "compilerOptions": { "composite": true, "rootDir": "src" } }"#,
            ),
            ("packages/lib/src/x.ts", "export const x = 1;\n"),
        ],
    );
    let s = scan(&dir);
    assert_eq!(
        deps(&s, "packages/app/src/main.ts"),
        vec![("packages/lib/src/x.ts".into(), EdgeKind::Import)]
    );
    assert!(s.graph.unresolved.is_empty(), "{:?}", s.graph.unresolved);
}

#[test]
fn a_deleted_file_is_joined_back_to_everything_that_referred_to_it() {
    let dir = repo(
        "deleted",
        &[
            ("package.json", r#"{ "workspaces": ["packages/*"] }"#),
            (
                "tsconfig.json",
                r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@app/*": ["app/*"] } } }"#,
            ),
            ("app/relative.ts", "import { gone } from './gone';\n"),
            ("app/alias.ts", "import { gone } from '@app/gone';\n"),
            ("app/spawn.test.ts", "const bin = 'tools/gone-cli.mjs';\n"),
            (
                "packages/lib/package.json",
                r#"{ "name": "lib", "exports": { ".": "./src/index.ts" } }"#,
            ),
            ("app/uses-lib.ts", "import { x } from 'lib';\n"),
            ("app/other.ts", "export const other = 1;\n"),
        ],
    );
    let mut s = scan(&dir);
    let deleted = [
        "app/gone.ts".to_string(),
        "tools/gone-cli.mjs".to_string(),
        "packages/lib/src/index.ts".to_string(),
    ];
    let ids = fairlead_lang::deleted::attach_deleted(&mut s, &Config::default().graph, &deleted);
    let importers = |id: u32| -> Vec<String> {
        let mut v: Vec<String> = s
            .graph
            .importers(id)
            .into_iter()
            .map(|(f, _)| s.graph.files[f as usize].clone())
            .collect();
        v.sort();
        v
    };
    assert_eq!(importers(ids[0]), ["app/alias.ts", "app/relative.ts"]);
    assert_eq!(importers(ids[1]), ["app/spawn.test.ts"]);
    assert_eq!(
        importers(ids[2]),
        ["app/uses-lib.ts"],
        "through the package edge"
    );
}
