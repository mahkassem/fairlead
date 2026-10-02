//! The Maven and Gradle extractors, and attribution of the classes they
//! name. The fixtures are written in the formats Surefire 3 and Gradle
//! print, as their reporters' sources build the lines; the Gradle file
//! names follow a public multi-module Kotlin build's layout.

use fairlead_replay::attribute::{attribute, Attribution, Repo};
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

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Each printed failure's file, or `?` when it's unattributed.
fn attributed(
    found: &[Printed],
    files: &[String],
    read: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let repo = Repo {
        files,
        packages: &[],
    };
    found
        .iter()
        .map(|p| match attribute(p, &repo, &read) {
            Attribution::File(f) => f,
            Attribution::Unattributed(_) => "?".into(),
        })
        .collect()
}

fn shop_files() -> Vec<String> {
    strings(&[
        "core/pom.xml",
        "core/src/main/java/com/acme/shop/Order.java",
        "core/src/test/java/com/acme/shop/OrderTest.java",
        "core/src/test/kotlin/com/acme/shop/PriceTest.kt",
        "client/src/test/kotlin/client/ClientTest.kt",
        "client/src/test/kotlin/client/Other.kt",
        "web/src/test/java/com/acme/shop/web/CartControllerTest.java",
        "web/src/test/java/com/acme/shop/web/SessionStoreTest.java",
        "web/src/test/java/com/acme/shop/web/RetryClientTest.java",
        "legacy/src/test/java/com/acme/shop/OrderTest.java",
    ])
}

fn shop_source(file: &str) -> Option<String> {
    let package = match file {
        "client/src/test/kotlin/client/ClientTest.kt" => "com.acme.shop.client",
        "client/src/test/kotlin/client/Other.kt" => "client",
        f if f.contains("/web/") => "com.acme.shop.web;",
        _ => "com.acme.shop;",
    };
    let body = if file.starts_with("legacy/") {
        "class OrderTest {}"
    } else {
        "class OrderTest { void totalIncludesTax() {} }"
    };
    Some(format!("package {package}\n\n{body}\n"))
}

#[test]
fn surefire_names_each_failed_test_once_and_not_its_flakes() {
    let found = extract(&Extractor::Maven, &fixture("maven-surefire3.log"));
    assert_eq!(
        found,
        [
            printed(
                "com/acme/shop/OrderTest.java",
                None,
                Some("totalIncludesTax")
            ),
            printed(
                "com/acme/shop/client/ClientTest.java",
                None,
                Some("retries a refused connection")
            ),
            printed(
                "com/acme/shop/client/ClientTest.java",
                None,
                Some("times out a slow read")
            ),
            printed("com/acme/shop/PriceTest.java", None, Some("rounds")),
            printed(
                "com/acme/shop/web/CartControllerTest.java",
                None,
                Some("checkoutIsRefused")
            ),
            printed("com/acme/shop/web/SessionStoreTest.java", None, None),
        ],
        "the closing lists repeat these, and the flaky RetryClientTest passed"
    );
}

#[test]
fn surefire_classes_attribute_to_java_and_kotlin_files_in_any_module() {
    let found = extract(&Extractor::Maven, &fixture("maven-surefire3.log"));
    assert_eq!(
        attributed(&found, &shop_files(), shop_source),
        [
            "core/src/test/java/com/acme/shop/OrderTest.java",
            "client/src/test/kotlin/client/ClientTest.kt",
            "client/src/test/kotlin/client/ClientTest.kt",
            "core/src/test/kotlin/com/acme/shop/PriceTest.kt",
            "web/src/test/java/com/acme/shop/web/CartControllerTest.java",
            "web/src/test/java/com/acme/shop/web/SessionStoreTest.java",
        ],
        "two modules hold an OrderTest in the same package; the test's name picks one, \
         and a Kotlin file may leave its package's folders out"
    );
}

#[test]
fn surefire_reads_the_dataset_excerpt_as_it_reads_the_whole_log() {
    let log = fixture("maven-surefire3.log");
    let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
    assert_eq!(
        extract(&Extractor::Maven, &excerpt),
        extract(&Extractor::Maven, &log)
    );
}

#[test]
fn surefire_2_names_the_method_before_its_class() {
    let log = "[INFO] Running com.acme.shop.OrderTest\n[ERROR] Tests run: 2, Failures: 1, Errors: 0, Skipped: 0, Time elapsed: 0.05 s <<< FAILURE! - in com.acme.shop.OrderTest\n[ERROR] totalIncludesTax(com.acme.shop.OrderTest)  Time elapsed: 0.01 s  <<< FAILURE!\njava.lang.AssertionError: expected:<107> but was:<100>\n[ERROR] Tests run: 1, Failures: 0, Errors: 1, Skipped: 0, Time elapsed: 0.2 s <<< FAILURE! - in com.acme.shop.web.SessionStoreTest\n";
    assert_eq!(
        extract(&Extractor::Maven, log),
        [
            printed(
                "com/acme/shop/OrderTest.java",
                None,
                Some("totalIncludesTax")
            ),
            printed("com/acme/shop/web/SessionStoreTest.java", None, None),
        ],
        "a failed class whose tests aren't named stands for itself"
    );
}

#[test]
fn a_surefire_summary_counts_only_for_a_class_nothing_above_named() {
    let log = "[ERROR] com.acme.shop.OrderTest.totalIncludesTax -- Time elapsed: 0.011 s <<< FAILURE!\n[INFO] Results:\n[ERROR] Failures: \n[ERROR]   OrderTest.totalIncludesTax:42 expected: <107.00> but was: <100.00>\n[ERROR]   CartControllerTest$WhenEmpty.checkoutIsRefused:88 Status expected:<409>\n[ERROR] Errors: \n[ERROR]   PriceTest>BaseTest.rounds:12 » Arithmetic Rounding necessary\n[ERROR]   Connection refused\n[INFO] \n[ERROR] Tests run: 9, Failures: 2, Errors: 2, Skipped: 0\n";
    assert_eq!(
        extract(&Extractor::Maven, log),
        [
            printed(
                "com/acme/shop/OrderTest.java",
                None,
                Some("totalIncludesTax")
            ),
            printed("CartControllerTest.java", None, Some("checkoutIsRefused")),
            printed("PriceTest.java", None, Some("rounds")),
        ]
    );
}

fn okhttp_files() -> Vec<String> {
    strings(&[
        "mockwebserver-deprecated/src/test/java/okhttp3/mockwebserver/MockWebServerTest.kt",
        "mockwebserver/src/test/java/mockwebserver3/MockWebServerTest.kt",
        "okhttp/src/jvmTest/kotlin/okhttp3/CallTest.kt",
        "okhttp/src/jvmTest/kotlin/okhttp3/OkHttpClientTest.kt",
        "okhttp/src/jvmTest/kotlin/okhttp3/internal/http2/Http2ConnectionTest.kt",
        "okhttp/src/jvmTest/kotlin/okhttp3/internal/ws/WebSocketReaderTest.kt",
        "regression-test/src/androidTest/java/okhttp/regression/compare/OkHttpClientTest.java",
        "samples/compare/src/test/kotlin/okhttp3/compare/OkHttpClientTest.kt",
    ])
}

#[test]
fn gradle_names_the_class_the_method_and_the_tasks_project() {
    let found = extract(&Extractor::Gradle, &fixture("gradle-junit5.log"));
    assert_eq!(
        found,
        [
            printed(
                "CallTest.java",
                Some("okhttp"),
                Some("cancelBeforeBodyIsRead")
            ),
            printed(
                "OkHttpClientTest.java",
                Some("okhttp"),
                Some("timeoutDefaults")
            ),
            printed(
                "MockWebServerTest.java",
                Some("mockwebserver"),
                Some("request with chunked body")
            ),
            printed(
                "OkHttpClientTest.java",
                Some("samples/compare"),
                Some("get")
            ),
            printed(
                "Http2ConnectionTest.java",
                Some("okhttp"),
                Some("dataAfterRstStreamIsIgnored")
            ),
            printed(
                "WebSocketReaderTest.java",
                Some("okhttp"),
                Some("closeCodes")
            ),
        ],
        "every failed task named its tests, so its closing report adds nothing"
    );
}

#[test]
fn gradle_simple_names_attribute_through_the_tasks_project() {
    let found = extract(&Extractor::Gradle, &fixture("gradle-junit5.log"));
    assert_eq!(
        attributed(&found, &okhttp_files(), |_| None),
        [
            "okhttp/src/jvmTest/kotlin/okhttp3/CallTest.kt",
            "okhttp/src/jvmTest/kotlin/okhttp3/OkHttpClientTest.kt",
            "mockwebserver/src/test/java/mockwebserver3/MockWebServerTest.kt",
            "samples/compare/src/test/kotlin/okhttp3/compare/OkHttpClientTest.kt",
            "okhttp/src/jvmTest/kotlin/okhttp3/internal/http2/Http2ConnectionTest.kt",
            "okhttp/src/jvmTest/kotlin/okhttp3/internal/ws/WebSocketReaderTest.kt",
        ]
    );
}

#[test]
fn gradle_reads_the_dataset_excerpt_as_it_reads_the_whole_log() {
    for name in ["gradle-junit5.log", "gradle-junit4-plain.log"] {
        let log = fixture(name);
        let excerpt = fairlead_replay::dataset::log_excerpt(&log).join("\n");
        assert_eq!(
            extract(&Extractor::Gradle, &excerpt),
            extract(&Extractor::Gradle, &log),
            "{name}"
        );
    }
}

#[test]
fn gradle_junit_4_names_qualified_and_nested_classes_and_a_silent_task() {
    let found = extract(&Extractor::Gradle, &fixture("gradle-junit4-plain.log"));
    assert_eq!(
        found,
        [
            printed(
                "com/acme/ledger/AccountTest.java",
                Some("core"),
                Some("depositRejectsNegative")
            ),
            printed(
                "com/acme/ledger/AccountTest.java",
                Some("core"),
                Some("limitIsEnforced")
            ),
            printed(
                "com/acme/ledger/LedgerSpec.java",
                Some("core"),
                Some("refuses new entries")
            ),
            printed(
                "com/acme/ledger/RatesTest.java",
                Some("core"),
                Some("testConvert")
            ),
            printed(
                "D:/a/ledger/ledger/reports/build/reports/tests/test/index.html",
                Some("reports"),
                None
            ),
        ]
    );
    let files = strings(&[
        "core/src/test/java/com/acme/ledger/AccountTest.java",
        "core/src/test/kotlin/com/acme/ledger/LedgerSpec.kt",
        "core/src/test/java/com/acme/ledger/RatesTest.java",
        "reports/src/test/java/com/acme/ledger/ReportTest.java",
    ]);
    assert_eq!(
        attributed(&found, &files, |_| None),
        [
            "core/src/test/java/com/acme/ledger/AccountTest.java",
            "core/src/test/java/com/acme/ledger/AccountTest.java",
            "core/src/test/kotlin/com/acme/ledger/LedgerSpec.kt",
            "core/src/test/java/com/acme/ledger/RatesTest.java",
            "?",
        ],
        "a task that named no test is a failure left unattributed"
    );
}

#[test]
fn a_shorter_tail_needs_the_file_to_declare_the_package() {
    let files = strings(&["app/src/test/kotlin/OrderTest.kt"]);
    let at = |source: &'static str| {
        attributed(
            &[printed("com/acme/OrderTest.java", None, None)],
            &files,
            move |_| Some(source.to_string()),
        )
    };
    assert_eq!(
        at("package com.acme\n"),
        ["app/src/test/kotlin/OrderTest.kt"]
    );
    assert_eq!(at("package org.other\n"), ["?"]);
}

#[test]
fn both_extractors_are_named_in_config() {
    assert!(matches!(
        Extractor::named("maven", None),
        Ok(Extractor::Maven)
    ));
    assert!(matches!(
        Extractor::named("gradle", None),
        Ok(Extractor::Gradle)
    ));
}

#[test]
fn gradle_names_a_test_file_that_fails_to_compile_and_not_main_code() {
    let found = extract(&Extractor::Gradle, &fixture("gradle-compile.log"));
    assert_eq!(
        found,
        [
            printed(
                "/home/runner/work/okhttp/okhttp/okhttp/src/jvmTest/kotlin/okhttp3/internal/TestAndSetTest.kt",
                Some("okhttp"),
                None
            ),
            printed(
                "/home/runner/work/okhttp/okhttp/regression-test/src/test/java/okhttp/regression/LetsEncryptTest.java",
                Some("regression-test"),
                None
            ),
        ],
        "a main source that fails to compile is no test's failure"
    );
    let files = strings(&[
        "okhttp/src/jvmTest/kotlin/okhttp3/internal/TestAndSetTest.kt",
        "regression-test/src/test/java/okhttp/regression/LetsEncryptTest.java",
        "mockwebserver/src/main/kotlin/mockwebserver3/MockWebServer.kt",
    ]);
    assert_eq!(
        attributed(&found, &files, |_| None),
        [
            "okhttp/src/jvmTest/kotlin/okhttp3/internal/TestAndSetTest.kt",
            "regression-test/src/test/java/okhttp/regression/LetsEncryptTest.java",
        ]
    );
    let excerpt = fairlead_replay::dataset::log_excerpt(&fixture("gradle-compile.log")).join("\n");
    assert_eq!(extract(&Extractor::Gradle, &excerpt), found);
}
