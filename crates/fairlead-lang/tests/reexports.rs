//! Named imports followed through re-exports, on synthetic repositories.

use std::fs;
use std::path::{Path, PathBuf};

use fairlead_core::config::Config;
use fairlead_lang::{build, Scan};

fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("fairlead-reexports-{name}-{}", std::process::id()));
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

/// The files a change to `changed` reaches, sorted.
fn reached(scan: &Scan, changed: &str) -> Vec<String> {
    let id = scan.graph.id(changed).unwrap();
    let mut out: Vec<String> = scan
        .graph
        .affected(&[id])
        .into_keys()
        .map(|f| scan.graph.files[f as usize].clone())
        .filter(|f| f != changed)
        .collect();
    out.sort();
    out
}

/// A barrel of two levels: `lib/index.ts` re-exports `./money` by name
/// (renamed) and everything in `./more`, which re-exports `./dates`.
const BARREL: &[(&str, &str)] = &[
    (
        "lib/index.ts",
        "export { cents as money } from './money';\nexport * from './more';\nexport const VERSION = 1;\n",
    ),
    ("lib/money.ts", "export const cents = 1;\n"),
    ("lib/more.ts", "export * from './dates';\nexport * from './time';\n"),
    ("lib/dates.ts", "export const today = 1;\nexport type Day = string;\n"),
    ("lib/time.ts", "export const now = 1;\n"),
];

fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    BARREL.iter().chain(extra).copied().collect()
}

#[test]
fn a_named_import_depends_on_the_file_that_declares_the_name() {
    let dir = repo(
        "named",
        &with(&[
            ("a.test.ts", "import { money } from './lib';\n"),
            ("b.test.ts", "import { today } from './lib';\n"),
            ("c.test.ts", "import type { Day } from './lib';\n"),
            ("d.test.ts", "import { VERSION } from './lib';\n"),
        ]),
    );
    let s = scan(&dir);
    assert_eq!(reached(&s, "lib/money.ts"), ["a.test.ts", "lib/index.ts"]);
    assert_eq!(
        reached(&s, "lib/dates.ts"),
        ["b.test.ts", "c.test.ts", "lib/index.ts", "lib/more.ts"]
    );
    assert_eq!(reached(&s, "lib/time.ts"), ["lib/index.ts", "lib/more.ts"]);
    // Each barrel on the way still counts for its own code.
    assert_eq!(
        reached(&s, "lib/index.ts"),
        ["a.test.ts", "b.test.ts", "c.test.ts", "d.test.ts"]
    );
    assert_eq!(
        reached(&s, "lib/more.ts"),
        ["b.test.ts", "c.test.ts", "lib/index.ts"]
    );
}

#[test]
fn what_cant_be_followed_by_name_keeps_the_whole_barrel() {
    let dir = repo(
        "whole",
        &with(&[
            ("named.test.ts", "import { money } from './lib';\n"),
            ("ns.test.ts", "import * as lib from './lib';\n"),
            ("default.test.ts", "import lib from './lib';\n"),
            ("side.test.ts", "import './lib';\n"),
            ("dynamic.test.ts", "const lib = await import('./lib');\n"),
            ("require.test.cjs", "const lib = require('./lib');\n"),
            (
                "mixed.test.ts",
                "import { money } from './lib';\nimport * as all from './lib';\n",
            ),
            (
                "missing.test.ts",
                "import { money, nothing } from './lib';\n",
            ),
            ("vendor.test.ts", "import { lodash } from './vendor';\n"),
            (
                "vendor.ts",
                "export * from 'some-package';\nexport * from './lib';\n",
            ),
            (
                "node_modules/some-package/package.json",
                r#"{ "name": "some-package", "main": "index.js" }"#,
            ),
            (
                "node_modules/some-package/index.js",
                "export const lodash = 1;\n",
            ),
        ]),
    );
    let s = scan(&dir);
    let want = [
        "default.test.ts",
        "dynamic.test.ts",
        "lib/index.ts",
        "lib/more.ts",
        "missing.test.ts",
        "mixed.test.ts",
        "ns.test.ts",
        "require.test.cjs",
        "side.test.ts",
        "vendor.test.ts",
        "vendor.ts",
    ];
    assert_eq!(reached(&s, "lib/time.ts"), want);
}

#[test]
fn a_cycle_of_re_exports_still_resolves() {
    let dir = repo(
        "cycle",
        &[
            ("a.ts", "export * from './b';\nexport const a = 1;\n"),
            ("b.ts", "export * from './a';\nexport * from './c';\n"),
            ("c.ts", "export const c = 1;\n"),
            ("d.ts", "export const d = 1;\n"),
            ("t.test.ts", "import { c } from './a';\n"),
            ("u.test.ts", "import { a } from './b';\n"),
        ],
    );
    let s = scan(&dir);
    assert_eq!(reached(&s, "c.ts"), ["a.ts", "b.ts", "t.test.ts"]);
    assert_eq!(reached(&s, "a.ts"), ["b.ts", "t.test.ts", "u.test.ts"]);
}

#[test]
fn turned_off_every_importer_of_a_barrel_depends_on_all_of_it() {
    let dir = repo(
        "off",
        &with(&[("a.test.ts", "import { money } from './lib';\n")]),
    );
    let mut config = Config::default();
    config.graph.reexports = false;
    let s = build(&dir, &config).unwrap();
    assert_eq!(
        reached(&s, "lib/time.ts"),
        ["a.test.ts", "lib/index.ts", "lib/more.ts"]
    );
}

#[test]
fn type_only_re_exports_left_out_keep_the_barrel() {
    let dir = repo(
        "types-off",
        &[
            (
                "index.ts",
                "export type { T } from './t';\nexport { v } from './v';\n",
            ),
            ("t.ts", "export type T = string;\n"),
            ("v.ts", "export const v = 1;\n"),
            ("a.test.ts", "import { T, v } from './index';\n"),
        ],
    );
    let mut config = Config::default();
    config.graph.type_imports = false;
    let s = build(&dir, &config).unwrap();
    assert_eq!(reached(&s, "v.ts"), ["a.test.ts", "index.ts"]);
    assert!(reached(&s, "t.ts").is_empty());
}
