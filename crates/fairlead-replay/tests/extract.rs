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

#[test]
fn bun_reads_a_second_run_in_the_same_job_after_the_first_summary() {
    let log = "packages/a/test/one.test.ts:\n(fail) one > breaks [1ms]\n\n1 tests failed:\n(fail) one > breaks [1ms]\n\npackages/b/test/two.test.ts:\n(fail) two > also breaks [1ms]\n\n1 tests failed:\n(fail) two > also breaks [1ms]\n";
    assert_eq!(
        extract(&Extractor::Bun, log),
        [
            printed("packages/a/test/one.test.ts", None, Some("breaks")),
            printed("packages/b/test/two.test.ts", None, Some("also breaks")),
        ]
    );
}

#[test]
fn bun_reads_a_header_whose_path_has_a_space() {
    let log = "packages/web/test/my cart.test.ts:\n(fail) cart > removes [2ms]\n";
    assert_eq!(
        extract(&Extractor::Bun, log),
        [printed(
            "packages/web/test/my cart.test.ts",
            None,
            Some("removes")
        )]
    );
}

#[test]
fn bun_counts_a_file_that_failed_to_load_as_that_files_failure() {
    let log = fixture("bun-load-error.log");
    let expected = [
        printed("packages/api/test/broken.test.ts", None, None),
        printed("packages/api/test/typo.test.ts", None, None),
    ];
    assert_eq!(extract(&Extractor::Bun, &log), expected);
    let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
    assert_eq!(extract(&Extractor::Bun, &excerpt), expected);
}

#[test]
fn bun_pins_an_unhandled_error_on_the_test_that_follows_it_and_ends_it_at_a_pass() {
    let log = "a.test.ts:\n# Unhandled error between tests\nerror: boom\n(fail) a > breaks [1ms]\nb.test.ts:\n(pass) b > one [1ms]\n# Unhandled error between tests\nerror: late\n(pass) b > two [1ms]\n";
    assert_eq!(
        extract(&Extractor::Bun, log),
        [
            printed("a.test.ts", None, Some("breaks")),
            printed("b.test.ts", None, None),
        ]
    );
}

#[test]
fn phpunit_names_the_test_file_from_its_frame_or_its_class_through_a_long_message() {
    let log = fixture("phpunit-laravel.log");
    let unit = "/home/runner/work/shop/shop/tests/Unit/Billing/PricingTest.php";
    let expected = [
        printed(unit, None, Some("test_an_empty_basket_is_priced_at_zero")),
        printed(unit, None, Some("test_total_adds_the_items")),
        printed(unit, None, Some("test_each_basket")),
        printed(
            "/home/runner/work/shop/shop/tests/Feature/HomeTest.php",
            None,
            Some("test_the_home_page_has_a_title"),
        ),
    ];
    assert_eq!(extract(&Extractor::Phpunit, &log), expected);
    let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
    assert_eq!(extract(&Extractor::Phpunit, &excerpt), expected);
}

#[test]
fn artisan_test_and_pest_name_the_test_file_even_when_the_error_is_thrown_elsewhere() {
    let unit = "tests/Unit/Billing/PricingTest.php";
    let artisan = [
        printed(unit, None, Some("total adds the items")),
        printed(unit, None, None),
        printed(unit, None, Some("each basket")),
        printed(
            "tests/Feature/HomeTest.php",
            None,
            Some("the home page has a title"),
        ),
    ];
    let cart = "tests/Unit/Cart/CartTest.php";
    let pest = [
        printed(cart, None, Some("prices a pair")),
        printed(cart, None, None),
        printed(cart, None, Some("adds with a dataset")),
        // Pest's own PHPUnit-style section after its report, the same file.
        printed(
            "/home/runner/work/shop/shop/tests/Unit/Cart/CartTest.php",
            None,
            None,
        ),
    ];
    for (name, expected) in [("artisan-test.log", &artisan[..]), ("pest.log", &pest[..])] {
        let log = fixture(name);
        assert_eq!(extract(&Extractor::Pest, &log), expected, "{name}");
        let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
        assert_eq!(
            extract(&Extractor::Pest, &excerpt),
            expected,
            "{name} excerpt"
        );
    }
}

#[test]
fn pest_also_reads_phpunits_format_which_artisan_test_prints_under_parallel() {
    let log = fixture("phpunit-laravel.log");
    assert_eq!(
        extract(&Extractor::Pest, &log),
        extract(&Extractor::Phpunit, &log)
    );
    assert_eq!(extract(&Extractor::Pest, &log).len(), 4);
}

#[test]
fn a_failed_line_cut_to_the_terminals_width_takes_its_class_from_the_fail_header() {
    let expected = [
        printed(
            "tests/Feature/Reports/ExportTest.php",
            None,
            Some("exports the monthly report as csv"),
        ),
        printed(
            "tests/Unit/Imports/InvoiceXmlExtractorTest.php",
            None,
            Some("extracts the payload from a pdf"),
        ),
        printed(
            "Feature/ClientPortal/PaymentMethodsTest.php",
            None,
            Some("another client is forbidden"),
        ),
    ];
    let log = fixture("collision-cut.log");
    for extractor in [Extractor::Pest, Extractor::Phpunit] {
        assert_eq!(extract(&extractor, &log), expected);
        let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
        assert_eq!(extract(&extractor, &excerpt), expected, "excerpt");
    }
}

#[test]
fn go_test_names_each_failed_test_by_its_logged_file_a_panic_frame_or_a_compile_error() {
    let price = "/example.com/shop/price/price_test.go";
    let expected = [
        printed("/example.com/shop/broken/broken_test.go", None, None),
        printed(
            "/home/runner/work/shop/shop/cart/cart_test.go",
            None,
            Some("TestItem"),
        ),
        printed(price, None, Some("TestTotal")),
        printed(price, None, Some("TestTable")),
    ];
    for name in ["go-plain.log", "go-verbose.log"] {
        let log = fixture(name);
        assert_eq!(extract(&Extractor::Go, &log), expected, "{name}");
        let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
        assert_eq!(
            extract(&Extractor::Go, &excerpt),
            expected,
            "{name} excerpt"
        );
    }
}

#[test]
fn pytest_reads_its_summary_verbose_lines_and_collection_errors() {
    let unit = "tests/unit/test_pricing.py";
    let broken = printed("tests/api/test_broken.py", None, None);
    let failed = [
        printed(unit, None, Some("test_total_adds")),
        printed(unit, None, Some("test_empty_basket")),
        printed(unit, None, Some("test_baskets")),
        printed(unit, None, Some("test_half")),
    ];
    let mut full = vec![broken.clone()];
    full.extend(failed.iter().cloned());
    let mut verbose = failed.to_vec();
    verbose.push(broken.clone());
    for (name, expected) in [
        ("pytest-full.log", full),
        ("pytest-v.log", verbose),
        ("pytest-default.log", vec![broken]),
    ] {
        let log = fixture(name);
        assert_eq!(extract(&Extractor::Pytest, &log), expected, "{name}");
        let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
        assert_eq!(
            extract(&Extractor::Pytest, &excerpt),
            expected,
            "{name} excerpt"
        );
    }
    let xdist = "[gw1] [ 50%] FAILED tests/unit/test_pricing.py::TestRounding::test_half[up] \n";
    assert_eq!(
        extract(&Extractor::Pytest, xdist),
        [printed(unit, None, Some("test_half"))]
    );
}

#[test]
fn artisan_test_parallel_on_a_real_laravel_app_is_read_as_phpunit() {
    let log = fixture("artisan-parallel.log");
    let file = "/home/runner/work/koel/koel/tests/Feature/CleanUrlsTest.php";
    assert_eq!(
        extract(&Extractor::Pest, &log),
        [
            printed(file, None, Some("serveTheAppForAScreenPathWhenEnabled")),
            printed(file, None, Some("tellTheAppWhetherCleanUrlsAreOn")),
        ]
    );
}
