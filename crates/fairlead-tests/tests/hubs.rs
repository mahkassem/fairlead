//! `graph hubs` counts what the planner's walk does: for every file, the
//! files whose walk reaches it and the unit tests its own walk reaches,
//! across modules, a cycle, conservative edges and a barrier.

mod common;

use std::collections::BTreeMap;

use common::*;
use fairlead_core::config::TestClass;
use fairlead_lang::build;
use fairlead_tests::hubs::{hubs, Why};
use fairlead_tests::modules::Modules;
use fairlead_tests::testfiles::discover;
use fairlead_tests::walk::{walk, Via};

const FILES: &[(&str, &str)] = &[
    (
        "package.json",
        r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    ),
    ("packages/core/package.json", r#"{ "name": "core" }"#),
    (
        "packages/core/src/loader.ts",
        "export const load = (n: string) => import(n);\n",
    ),
    (
        "packages/core/src/store.ts",
        "import { load } from './loader';\nimport { b } from './b';\nexport const s = load;\n",
    ),
    (
        "packages/core/src/b.ts",
        "import { s } from './store';\nexport const b = s;\n",
    ),
    (
        "packages/core/src/gate.ts",
        "import { s } from './store';\n",
    ),
    ("packages/core/src/leaf.ts", "export const leaf = 1;\n"),
    (
        "packages/core/test/store.test.ts",
        "import { s } from '../src/store';\n",
    ),
    ("packages/core/test/gate.test.ts", "import '../src/gate';\n"),
    ("packages/web/package.json", r#"{ "name": "web" }"#),
    (
        "packages/web/src/page.ts",
        "import '../../core/src/gate';\nimport './missing';\n",
    ),
    ("packages/web/src/view.ts", "export const v = 1;\n"),
    ("packages/web/test/page.test.ts", "import '../src/page';\n"),
    ("packages/web/test/view.test.ts", "import '../src/view';\n"),
];

#[test]
fn hub_counts_match_a_walk_from_every_file() {
    let dir = repo("hubs-walk", FILES);
    let config = config(&format!(
        "{VITEST}\n[graph]\nbarrier = [\"packages/core/src/gate.ts\"]\n"
    ));
    let scan = build(&dir, &config).unwrap();
    let graph = &scan.graph;
    let modules = Modules::discover(&scan.tree, &scan.packages, &config.modules).unwrap();
    let found = discover(&scan.tree, &config, &modules).unwrap();
    let units: Vec<u32> = found
        .tests
        .iter()
        .filter(|t| t.class == TestClass::Unit)
        .filter_map(|t| graph.id(&t.path))
        .collect();
    let n = graph.files.len() as u32;
    let walks: Vec<_> = (0..n)
        .map(|id| walk(graph, &modules, vec![(id, Via::Start)]))
        .collect();
    let mut want = BTreeMap::new();
    for f in 0..n {
        let path = graph.files[f as usize].clone();
        if graph.is_barrier(f) || found.tests.iter().any(|t| t.path == path) {
            continue;
        }
        let tests = units
            .iter()
            .filter(|&&t| walks[f as usize].reached.contains_key(&t))
            .count() as u32;
        let feeders = (0..n)
            .filter(|&g| g != f && walks[g as usize].reached.contains_key(&f))
            .count() as u32;
        if tests * feeders > 0 {
            want.insert(path, (feeders, tests));
        }
    }
    let found = hubs(&scan, &config, usize::MAX).unwrap();
    let got: BTreeMap<String, (u32, u32)> = found
        .hubs
        .iter()
        .map(|h| (h.path.clone(), (h.feeders, h.tests)))
        .collect();
    assert_eq!(got, want);
    assert!(want.len() > 3, "{want:?}");
    let scores: Vec<u64> = found.hubs.iter().map(|h| h.score()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
    // The web page's unresolved import makes it depend on its whole module.
    let page = found
        .hubs
        .iter()
        .find(|h| h.path == "packages/web/src/page.ts")
        .unwrap();
    assert!(
        matches!(
            &page.why,
            Why::Own {
                kind: "unresolved-import",
                ..
            }
        ),
        "{:?}",
        page.why
    );
}

#[test]
fn on_a_tie_across_a_cycle_the_file_more_files_import_comes_first() {
    let dir = repo(
        "hubs-tie",
        &[
            ("package.json", r#"{ "private": true }"#),
            ("src/b.ts", "import './z';\n"),
            ("src/z.ts", "import './b';\n"),
            ("src/x.ts", "import './z';\n"),
            ("src/y.ts", "import './z';\n"),
            ("test/x.test.ts", "import '../src/x';\n"),
            ("test/y.test.ts", "import '../src/y';\n"),
        ],
    );
    let config = config(VITEST);
    let scan = build(&dir, &config).unwrap();
    let found = hubs(&scan, &config, 2).unwrap();
    let ranked: Vec<(&str, u64)> = found
        .hubs
        .iter()
        .map(|h| (h.path.as_str(), h.score()))
        .collect();
    assert_eq!(ranked, [("src/z.ts", 2), ("src/b.ts", 2)]);
}
