//! Coverage maps: real PHPUnit and coverage.py reports turned into test to
//! file edges, joining the static graph where a container or a dynamic
//! import hides the dependency.

use std::fs;
use std::path::{Path, PathBuf};

use fairlead_core::config::{Config, Coverage};
use fairlead_core::coverage::CoverageMap;
use fairlead_lang::coverage::{from_coverage_py, from_phpunit_xml, Tests};
use fairlead_lang::{build, EdgeKind, Scan};

fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-cov-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).unwrap();
    for (path, text) in files {
        let full = dir.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }
    dir
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/coverage")
        .join(name)
}

fn depends(scan: &Scan, from: &str, to: &str) -> bool {
    let (a, b) = (scan.graph.id(from).unwrap(), scan.graph.id(to).unwrap());
    scan.graph.affected(&[b]).contains_key(&a)
}

fn with_map(dir: &Path, tests: &Tests) -> Scan {
    let map = CoverageMap {
        version: 1,
        commit: "abc".into(),
        created: "2026-09-28".into(),
        source: "test".into(),
        tests: tests
            .iter()
            .map(|(t, f)| (t.clone(), f.iter().cloned().collect()))
            .collect(),
    };
    fs::write(
        dir.join("coverage.json"),
        serde_json::to_string(&map).unwrap(),
    )
    .unwrap();
    let mut config = Config::default();
    config.graph.coverage = Some(Coverage {
        map: "coverage.json".into(),
        max_age_days: 14,
    });
    build(dir, &config).unwrap()
}

const PHP: &[(&str, &str)] = &[
    (
        "composer.json",
        r#"{"autoload":{"psr-4":{"App\\":"src/"},"files":["src/helpers.php"]},"autoload-dev":{"psr-4":{"Tests\\":"tests/"}}}"#,
    ),
    ("src/Billing/Pricing.php", "<?php\nnamespace App\\Billing;\n\nclass Pricing { public function total(array $i): int { return (new Rounding)->round(array_sum($i)); } }\n"),
    ("src/Billing/Rounding.php", "<?php\nnamespace App\\Billing;\n\nclass Rounding { public function round(int $x): int { return $x; } }\n"),
    ("src/Billing/Tax.php", "<?php\nnamespace App\\Billing;\n\nclass Tax { public function rate(): float { return 0.15; } }\n"),
    ("src/helpers.php", "<?php\n\nfunction app_container() { return new class { public function make(string $n) { $c = 'App\\\\Billing\\\\' . ucfirst($n); return new $c; } }; }\n"),
    ("tests/Unit/PricingTest.php", "<?php\nnamespace Tests\\Unit;\n\nuse App\\Billing\\Pricing;\nuse PHPUnit\\Framework\\TestCase;\n\nclass PricingTest extends TestCase { public function test_total(): void { $this->assertSame(3, (new Pricing)->total([1, 2])); } }\n"),
    ("tests/Unit/TaxTest.php", "<?php\nnamespace Tests\\Unit;\n\nuse PHPUnit\\Framework\\TestCase;\n\nclass TaxTest extends TestCase { public function test_rate(): void { $this->assertSame(0.15, app_container()->make('tax')->rate()); } }\n"),
];

#[test]
fn a_phpunit_report_joins_a_test_to_the_class_only_the_container_names() {
    let dir = repo("php", PHP);
    let plain = build(&dir, &Config::default()).unwrap();
    assert!(
        !depends(&plain, "tests/Unit/TaxTest.php", "src/Billing/Tax.php"),
        "the static graph can't see the container"
    );
    let (tests, unknown) =
        from_phpunit_xml(&fixture("phpunit-xml"), &plain.tree, &plain.autoload).unwrap();
    assert!(unknown.is_empty(), "{unknown:?}");
    let files = |t: &str| tests[t].iter().cloned().collect::<Vec<_>>();
    assert_eq!(
        files("tests/Unit/TaxTest.php"),
        ["src/Billing/Tax.php", "src/helpers.php"]
    );
    assert_eq!(
        files("tests/Unit/PricingTest.php"),
        ["src/Billing/Pricing.php", "src/Billing/Rounding.php"]
    );
    let joined = with_map(&dir, &tests);
    assert!(depends(
        &joined,
        "tests/Unit/TaxTest.php",
        "src/Billing/Tax.php"
    ));
    let tax = joined.graph.id("src/Billing/Tax.php").unwrap();
    let test = joined.graph.id("tests/Unit/TaxTest.php").unwrap();
    assert!(joined
        .graph
        .dependencies(test)
        .contains(&(tax, EdgeKind::Coverage)));
    let report = joined.coverage.as_ref().unwrap();
    assert_eq!((report.tests, report.edges, report.ignored), (2, 4, 0));
}

#[test]
fn a_coverage_py_report_joins_a_test_to_the_module_it_imports_by_name() {
    let dir = repo(
        "py",
        &[
            ("pyproject.toml", "[project]\nname = \"shop\"\n"),
            ("src/shop/__init__.py", ""),
            ("src/shop/pricing.py", "def total(items):\n    return sum(items)\n"),
            ("src/shop/registry.py", "import importlib\n\ndef plugin(name):\n    return importlib.import_module(\"shop.\" + name)\n"),
            ("src/shop/tax.py", "RATE = 0.15\n\ndef rate():\n    return RATE\n"),
            ("tests/unit/test_tax.py", "from shop.registry import plugin\n\ndef test_rate():\n    assert plugin(\"tax\").rate() == 0.15\n"),
            ("tests/unit/test_pricing.py", "from shop.pricing import total\n\ndef test_total():\n    assert total([1, 2]) == 3\n"),
            ("tests/unit/test_ok.py", "def test_ok():\n    assert True\n"),
        ],
    );
    let plain = build(&dir, &Config::default()).unwrap();
    assert!(!depends(
        &plain,
        "tests/unit/test_tax.py",
        "src/shop/tax.py"
    ));
    let text = fs::read_to_string(fixture("coverage-py.json")).unwrap();
    let (tests, unknown) = from_coverage_py(&text, &plain.tree).unwrap();
    assert!(unknown.is_empty(), "{unknown:?}");
    assert_eq!(
        tests["tests/unit/test_tax.py"]
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        ["src/shop/registry.py", "src/shop/tax.py"],
        "a line run outside any test (an import) counts for none"
    );
    let joined = with_map(&dir, &tests);
    assert!(depends(
        &joined,
        "tests/unit/test_tax.py",
        "src/shop/tax.py"
    ));
    let no_contexts = r#"{"files": {"src/shop/tax.py": {"contexts": {"1": [""]}}}}"#;
    assert!(from_coverage_py(no_contexts, &plain.tree)
        .unwrap_err()
        .contains("--cov-context=test"));
}

#[test]
fn an_unreadable_map_leaves_the_static_graph_and_says_why() {
    let dir = repo("bad", PHP);
    fs::write(dir.join("coverage.json"), "{\"version\": 2}").unwrap();
    let mut config = Config::default();
    config.graph.coverage = Some(Coverage {
        map: "coverage.json".into(),
        max_age_days: 14,
    });
    let scan = build(&dir, &config).unwrap();
    let report = scan.coverage.unwrap();
    assert!(
        report.error.unwrap().contains("coverage map"),
        "missing fields"
    );
    assert_eq!(report.edges, 0);
}
