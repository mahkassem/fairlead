//! Rule edges and the walk barrier, on a server whose API tests reach their
//! area over the network: every area imports the shared server module, and
//! the server imports every area.

mod common;

use common::*;
use fairlead_core::config::Config;
use fairlead_lang::build;
use fairlead_tests::explain::explain;

const SERVER: &[(&str, &str)] = &[
    ("package.json", r#"{ "private": true }"#),
    (
        "app/src/http/server.ts",
        "import '../orders/routes';\nimport '../billing/routes';\nimport '../people/routes';\n",
    ),
    ("app/src/http/auth.ts", "export const requireUser = 1;\n"),
    (
        "app/src/orders/routes.ts",
        "import '../http/server';\nimport { requireUser } from '../http/auth';\nimport { total } from '../billing/money';\n",
    ),
    (
        "app/src/billing/routes.ts",
        "import { requireUser } from '../http/auth';\n",
    ),
    (
        "app/src/billing/money.ts",
        "import { rate } from './rates';\nexport const total = rate;\n",
    ),
    ("app/src/billing/rates.ts", "export const rate = 1;\n"),
    ("app/src/billing/legacy.ts", "export const legacy = 1;\n"),
    ("app/src/people/routes.ts", "import '../http/server';\n"),
    ("app/test/api/orders.test.ts", "export {};\n"),
    ("app/test/api/billing.test.ts", "export {};\n"),
    ("app/test/api/billing-refunds.test.ts", "export {};\n"),
    ("app/test/api/people.test.ts", "export {};\n"),
    (
        "app/test/unit/money.test.ts",
        "import { total } from '../../src/billing/money';\n",
    ),
];

const EDGES: &str = r#"
[graph]
barrier = ["app/src/http/**"]

[[graph.edges]]
from = "app/test/api/{area}{,-*}.test.ts"
to = ["app/src/{area}/**"]
"#;

fn with_edges(barrier: bool) -> Config {
    let text = if barrier {
        EDGES.to_string()
    } else {
        EDGES.replace(r#"barrier = ["app/src/http/**"]"#, "")
    };
    config(&format!("{VITEST}{text}"))
}

#[test]
fn a_shared_area_selects_its_own_tests_its_importers_tests_and_the_split_siblings() {
    let dir = repo("edges-shared", SERVER);
    let plan = run(
        &dir,
        &with_edges(true),
        vec![modified("app/src/billing/rates.ts")],
    );
    assert_eq!(
        tests(&plan),
        [
            "app/test/api/billing-refunds.test.ts",
            "app/test/api/billing.test.ts",
            "app/test/api/orders.test.ts",
            "app/test/unit/money.test.ts",
        ]
    );
}

#[test]
fn a_leaf_area_selects_only_its_own_tests_because_the_server_is_a_barrier() {
    let dir = repo("edges-leaf", SERVER);
    let plan = run(
        &dir,
        &with_edges(true),
        vec![modified("app/src/people/routes.ts")],
    );
    assert_eq!(tests(&plan), ["app/test/api/people.test.ts"]);
}

#[test]
fn without_the_barrier_the_server_carries_a_leaf_change_into_every_area_importing_it() {
    let dir = repo("edges-no-barrier", SERVER);
    let plan = run(
        &dir,
        &with_edges(false),
        vec![modified("app/src/people/routes.ts")],
    );
    assert_eq!(
        tests(&plan),
        ["app/test/api/orders.test.ts", "app/test/api/people.test.ts"]
    );
}

#[test]
fn a_changed_barrier_file_reaches_nothing_and_is_left_to_the_unreached_rule() {
    let dir = repo("edges-barrier-change", SERVER);
    let plan = run(
        &dir,
        &with_edges(true),
        vec![modified("app/src/http/auth.ts")],
    );
    assert!(plan
        .unreached
        .iter()
        .any(|u| u.path == "app/src/http/auth.ts"));
}

#[test]
fn a_deleted_file_nothing_imports_still_selects_its_area_through_the_rule() {
    let dir = repo("edges-deleted", SERVER);
    std::fs::remove_file(dir.join("app/src/billing/legacy.ts")).unwrap();
    let plan = run(
        &dir,
        &with_edges(true),
        vec![deleted("app/src/billing/legacy.ts")],
    );
    assert_eq!(
        tests(&plan),
        [
            "app/test/api/billing-refunds.test.ts",
            "app/test/api/billing.test.ts"
        ]
    );
}

#[test]
fn explain_names_the_barrier_that_kept_a_test_out() {
    let dir = repo("edges-explain", SERVER);
    let config = with_edges(true);
    let plan = run(&dir, &config, vec![modified("app/src/people/routes.ts")]);
    let scan = build(&dir, &config).unwrap();
    let text = explain(&plan, &scan, &config, "app/test/api/orders.test.ts").unwrap();
    assert!(
        text.contains("only through app/src/http/server.ts"),
        "{text}"
    );
}
