//! Deleted and renamed files: their importers still count.

mod common;

use common::*;

const FILES: &[(&str, &str)] = &[
    ("package.json", r#"{ "workspaces": ["packages/*"] }"#),
    ("tsconfig.base.json", "{}"),
    (
        "packages/app/tsconfig.json",
        r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@app/*": ["src/*"] } } }"#,
    ),
    ("packages/app/package.json", r#"{ "name": "app" }"#),
    (
        "packages/app/src/relative.ts",
        "import { gone } from './gone';\n",
    ),
    (
        "packages/app/src/alias.ts",
        "import { gone } from '@app/gone';\n",
    ),
    (
        "packages/app/test/relative.test.ts",
        "import '../src/relative';\n",
    ),
    (
        "packages/app/test/alias.test.ts",
        "import '../src/alias';\n",
    ),
    (
        "packages/cli/package.json",
        r#"{ "name": "cli", "exports": { ".": "./src/index.ts" } }"#,
    ),
    ("packages/cli/bin/new-name.mjs", "console.log(1);\n"),
    ("packages/cli/test/other.test.ts", "export {};\n"),
    (
        "e2e/spawn.test.ts",
        "const bin = 'packages/cli/bin/old-name.mjs';\n",
    ),
    ("e2e/uses-cli.test.ts", "import { run } from 'cli';\n"),
];

#[test]
fn a_deleted_file_still_selects_the_tests_of_its_relative_and_alias_importers() {
    let dir = repo("del-imports", FILES);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![deleted("packages/app/src/gone.ts")],
    );
    assert_eq!(
        tests(&plan),
        [
            "packages/app/test/alias.test.ts",
            "packages/app/test/relative.test.ts"
        ]
    );
    assert!(plan.unreached.is_empty());
}

#[test]
fn renaming_a_spawned_file_selects_the_test_that_names_its_old_path() {
    let dir = repo("del-literal", FILES);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![renamed(
            "packages/cli/bin/old-name.mjs",
            "packages/cli/bin/new-name.mjs",
        )],
    );
    assert!(
        tests(&plan).contains(&"e2e/spawn.test.ts"),
        "{:?}",
        tests(&plan)
    );
}

#[test]
fn deleting_a_packages_entry_selects_the_tests_that_import_the_package() {
    let dir = repo("del-exports", FILES);
    let plan = run(
        &dir,
        &config(VITEST),
        vec![deleted("packages/cli/src/index.ts")],
    );
    assert!(
        tests(&plan).contains(&"e2e/uses-cli.test.ts"),
        "{:?}",
        tests(&plan)
    );
}
