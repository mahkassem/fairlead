//! The extractors on short excerpts of real public CI logs.

use fairlead_replay::extract::{extract, Extractor, Printed};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn printed(path: &str, project: Option<&str>, title: Option<&str>) -> Printed {
    Printed {
        path: path.into(),
        project: project.map(Into::into),
        title: title.map(Into::into),
    }
}

#[test]
fn vitest_project_lines_give_the_project_the_path_and_the_test() {
    let found = extract(&Extractor::Vitest, &fixture("vitest-projects.log"));
    assert_eq!(
        found,
        [
            printed("test/Snapshot.test.ts", Some("@effect/api-diff"), Some("extracts declarations, overloads, namespaces, re-exports, and canonical types deterministically")),
            printed("test/Snapshot.test.ts", Some("@effect/api-diff"), Some("normalizes union order in structural fingerprints")),
            printed("test/Pool.test.ts", Some("effect"), Some("finalizer is called for failed allocations")),
        ]
    );
}

#[test]
fn vitest_skips_failures_printed_inside_another_runs_assertion_diff() {
    let found = extract(&Extractor::Vitest, &fixture("vitest-nested.log"));
    assert_eq!(
        found,
        [
            printed(
                "test/mocking.test.ts",
                Some("main"),
                Some("importOriginal for virtual modules (playwright)")
            ),
            printed(
                "specs/bail-out.test.ts",
                None,
                Some("exits gracefully when the browser connection is closed while cancelling")
            ),
        ]
    );
}

#[test]
fn jest_lines_behind_a_workspace_prefix_take_their_title_from_the_bullet() {
    let found = extract(&Extractor::Jest, &fixture("jest-chunk.log"));
    assert_eq!(
        found,
        [
            printed(
                "test/index.ts",
                None,
                Some("exit with error on INT signal from child")
            ),
            printed(
                "test/install/modulesDir.ts",
                None,
                Some("uses the directory")
            ),
        ],
        "the Rust test runner's FAIL line names no test file"
    );
}

#[test]
fn a_package_prefix_becomes_the_project() {
    let log = "packages/core test: FAIL test/a.test.ts (1.2 s)\n";
    assert_eq!(
        extract(&Extractor::Jest, log),
        [printed("test/a.test.ts", Some("packages/core"), None)]
    );
}

#[test]
fn a_custom_pattern_needs_a_file_group() {
    assert!(Extractor::named("regex", Some(r"^ERR (?P<file>\S+)$")).is_ok());
    assert!(Extractor::named("regex", Some(r"^ERR (\S+)$")).is_err());
    let re = Extractor::named("regex", Some(r"^ERR (?P<file>\S+)$")).unwrap();
    assert_eq!(
        extract(&re, "ERR tests/x.spec.ts\nok\n"),
        [printed("tests/x.spec.ts", None, None)]
    );
}

#[test]
fn bun_attributes_each_failure_to_the_header_it_sits_under_and_stops_at_the_summary() {
    assert_eq!(
        extract(&Extractor::Bun, &fixture("bun-parallel.log")),
        [
            printed(
                "packages/api/test/refunds.test.ts",
                None,
                Some("a second refund of the same order is refused")
            ),
            printed("packages/api/test/invoices.test.ts", None, None),
        ]
    );
}

#[test]
fn bun_reads_the_plain_header_printed_outside_github() {
    let log =
        "packages/web/test/cart.test.tsx:\n(pass) cart > adds [1ms]\n(fail) cart > removes [2ms]\n";
    assert_eq!(
        extract(&Extractor::Bun, log),
        [printed(
            "packages/web/test/cart.test.tsx",
            None,
            Some("removes")
        )]
    );
}

#[test]
fn bun_reads_the_same_failures_from_the_dataset_excerpt_as_from_the_whole_log() {
    let log = fixture("bun-parallel.log");
    let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
    assert_eq!(
        extract(&Extractor::Bun, &excerpt),
        extract(&Extractor::Bun, &log)
    );
}
