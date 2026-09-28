//! Coverage maps from the command line: a coverage.py report imported with
//! `coverage import` selects the test a dynamic import hides, and a map
//! that is old or unreadable leaves a warning on the plan.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fairlead_in(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.output().unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-coverage-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(
        &dir,
        "fairlead.toml",
        "[tests]\nmatch = [\"tests/**/test_*.py\"]\n\n[[tests.runners]]\nid = \"pytest\"\nmatch = [\"tests/**\"]\ncommand = [\"pytest\", \"{files}\"]\n\n[graph.coverage]\nmap = \".fairlead/coverage.json\"\n",
    );
    write(&dir, "pyproject.toml", "[project]\nname = \"shop\"\n");
    write(&dir, "src/shop/__init__.py", "");
    write(
        &dir,
        "src/shop/pricing.py",
        "def total(items):\n    return sum(items)\n",
    );
    write(&dir, "src/shop/registry.py", "import importlib\n\ndef plugin(name):\n    return importlib.import_module(\"shop.\" + name)\n");
    write(&dir, "src/shop/tax.py", "RATE = 0.15\n");
    write(
        &dir,
        "tests/unit/test_tax.py",
        "from shop.registry import plugin\n",
    );
    write(
        &dir,
        "tests/unit/test_pricing.py",
        "from shop.pricing import total\n",
    );
    std::fs::create_dir_all(dir.join(".fairlead")).unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    write(&dir, "src/shop/tax.py", "RATE = 0.16\n");
    dir
}

fn plan_in(dir: &Path) -> serde_json::Value {
    let out = fairlead_in(dir, &["plan", "--base", "main", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn tests_of(plan: &serde_json::Value) -> Vec<String> {
    plan["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap().to_string())
        .collect()
}

fn warning<'a>(plan: &'a serde_json::Value, code: &str) -> Option<&'a serde_json::Value> {
    plan["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["code"] == code)
}

#[test]
fn an_imported_coverage_py_report_selects_the_test_a_dynamic_import_hides() {
    let dir = project("import");
    let report = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fairlead-lang/tests/fixtures/coverage/coverage-py.json");
    let out = fairlead_in(
        &dir,
        &[
            "coverage",
            "import",
            "--format",
            "coverage-py",
            report.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("coverage map: 2 tests ran 3 files"),
        "{stdout}"
    );
    let map: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(".fairlead/coverage.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        (map["version"].as_u64(), map["source"].as_str()),
        (Some(1), Some("coverage-py"))
    );
    assert_eq!(
        map["commit"].as_str().unwrap().len(),
        40,
        "stamped with HEAD"
    );
    let plan = plan_in(&dir);
    assert_eq!(tests_of(&plan), ["tests/unit/test_tax.py"], "{plan}");
    assert!(warning(&plan, "coverage-stale").is_none());
    let stats = fairlead_in(&dir, &["graph", "stats"]);
    assert!(String::from_utf8_lossy(&stats.stdout)
        .contains("coverage map .fairlead/coverage.json: 3 edges from 2 tests, coverage-py"));
}

#[test]
fn an_old_map_and_an_unreadable_one_each_leave_a_warning() {
    let dir = project("stale");
    write(
        &dir,
        ".fairlead/coverage.json",
        r#"{"version":1,"commit":"0123456789abcdef","created":"2020-01-01","source":"coverage-py","tests":{"tests/unit/test_tax.py":["src/shop/tax.py"]}}"#,
    );
    let plan = plan_in(&dir);
    assert_eq!(
        tests_of(&plan),
        ["tests/unit/test_tax.py"],
        "an old map still counts"
    );
    let stale = warning(&plan, "coverage-stale").expect("a coverage-stale warning");
    assert!(
        stale["message"]
            .as_str()
            .unwrap()
            .contains("2020-01-01, commit 0123456789ab"),
        "{stale}"
    );
    write(&dir, ".fairlead/coverage.json", "not json");
    let plan = plan_in(&dir);
    assert!(warning(&plan, "coverage-unreadable").is_some(), "{plan}");
    assert_eq!(
        plan["unreached"][0]["path"], "src/shop/tax.py",
        "only the static graph, which nothing reaches the change through, so it widens"
    );
}
