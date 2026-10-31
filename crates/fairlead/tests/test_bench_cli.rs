//! `fairlead test bench` against a stub `bun` first on the PATH, which
//! sleeps a set time per mode and fails one file only under `--parallel`,
//! so which mode wins and which file is order-dependent are fixed; then the
//! result as `doctor --score`, `insights --suggest` and a refusal read it.
//! The stubs are shell scripts, so the runs are Unix-only; the rules behind
//! them are unit tests in `test_bench.rs`, which run everywhere.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn fairlead(dir: &Path, stubs: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["SYSTEMROOT", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    let mut path = vec![stubs.to_path_buf()];
    path.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    cmd.env("PATH", std::env::join_paths(path).unwrap());
    cmd.output().unwrap()
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

#[cfg(unix)]
fn stub(dir: &Path, name: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Serial takes 0.6 s, `--parallel` 0.05 s and `--no-isolate` 0.25 s; under
/// `--parallel` alone `test/leaky.test.ts` fails, as a test sharing state would.
#[cfg(unix)]
const BUN: &str = r#"
case "$1" in
  --version) echo "$STUB_VERSION"; exit 0 ;;
  check) sleep 0.05; exit 0 ;;
esac
shift
echo "$*" >> "$STUB_LOG"
mode=serial; out=""; files=""
for a in "$@"; do
  case "$a" in
    --parallel) [ "$mode" = serial ] && mode=parallel ;;
    --no-isolate) mode=no-isolate ;;
    --reporter-outfile=*) out="${a#--reporter-outfile=}" ;;
    -*) ;;
    *) files="$files $a" ;;
  esac
done
case "$mode" in serial) sleep 0.6 ;; parallel) sleep 0.05 ;; *) sleep 0.25 ;; esac
{
  echo '<testsuites>'
  for f in $files; do
    n=0
    if [ "$mode" = parallel ] && [ "$f" = test/leaky.test.ts ]; then n=1; fi
    echo "<testsuite name=\"$f\" file=\"$f\" failures=\"$n\"></testsuite>"
  done
  echo '</testsuites>'
} > "$out"
"#;

#[cfg_attr(not(unix), allow(unused_variables))]
fn repo(name: &str, version: &str, runner: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("fairlead-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let stubs = dir.join("stubs");
    std::fs::create_dir_all(&stubs).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    write(&repo, "test/a.test.ts", "");
    write(&repo, "test/leaky.test.ts", "");
    write(&repo, "test/slow.test.ts", "");
    write(&repo, "test/manual.test.ts", "");
    write(
        &repo,
        "fairlead.toml",
        &format!("[[tests.runners]]\nid = \"bun\"\nmatch = [\"test/**\"]\ncommand = {runner}\n\n[[tests.classes]]\nclass = \"demand\"\nmatch = [\"test/manual.test.ts\"]\n\n[[checks]]\nid = \"types\"\ncommand = [\"tsc\", \"-p\", \"tsconfig.json\"]\npaths = [\"**/*.ts\"]\n"),
    );
    #[cfg(unix)]
    {
        let log = dir.join("bun.log");
        stub(
            &stubs,
            "bun",
            &format!("STUB_VERSION={version}\nSTUB_LOG={}\n{BUN}", log.display()),
        );
        stub(&stubs, "tsc", "sleep 0.3\nexit 0\n");
    }
    (repo, stubs)
}

fn json(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn item<'a>(score: &'a Value, id: &str) -> &'a Value {
    score["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == id)
        .unwrap_or_else(|| panic!("no item {id}"))
}

#[cfg(unix)]
#[test]
fn the_fastest_mode_an_order_dependent_file_passes_in_is_recommended_and_read_back() {
    let (repo, stubs) = repo("pick", "1.4.4", r#"["bun", "test", "{files}"]"#);
    let b = json(&fairlead(&repo, &stubs, &["test", "bench", "--json"]));
    let means: Vec<(String, f64)> = b["modes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["mode"].as_str().unwrap().to_string(),
                m["mean_seconds"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(means.len(), 3, "{b}");
    assert!(
        means[1].1 < means[2].1 && means[2].1 < means[0].1,
        "{means:?}"
    );
    assert_eq!(b["files"], 3, "a demand test isn't timed");
    assert_eq!(b["order_dependent"][0]["file"], "test/leaky.test.ts");
    assert_eq!(
        b["order_dependent"][0]["failed_in"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "it failed in both --parallel runs: {b}"
    );
    assert_eq!(b["recommended"], "parallel-no-isolate", "{b}");
    assert_eq!(b["configured"], "serial");
    assert_eq!(b["typecheck"]["bun_check"][1], "check");
    assert_eq!(b["typecheck"]["bun_check"][3], "tsconfig.json");
    let tsc = b["typecheck"]["tsc_runs"][0]["seconds"].as_f64().unwrap();
    let bun = b["typecheck"]["bun_check_runs"][0]["seconds"]
        .as_f64()
        .unwrap();
    assert!(bun < tsc, "{b}");
    let log = std::fs::read_to_string(repo.parent().unwrap().join("bun.log")).unwrap();
    assert_eq!(log.lines().count(), 6, "three modes, twice each: {log}");
    assert!(log.lines().all(|l| l.contains("--reporter=junit")), "{log}");

    let text = fairlead(&repo, &stubs, &["test", "bench", "--runs", "1"]);
    let text = String::from_utf8_lossy(&text.stdout).into_owned();
    assert!(
        text.contains("order-dependent: test/leaky.test.ts failed in bun test --parallel, run 1"),
        "{text}"
    );
    assert!(
        text.contains("recommend: bun test --parallel --no-isolate"),
        "{text}"
    );
    assert!(text.contains("wrote .fairlead/test-bench.json"), "{text}");
    assert!(repo.join(".fairlead/test-bench.json").is_file());

    let score = json(&fairlead(&repo, &stubs, &["doctor", "--score", "--json"]));
    let bench = item(&score, "bench");
    assert_eq!(bench["points"], 0, "{bench}");
    assert!(
        bench["fix"]
            .as_str()
            .unwrap()
            .contains("fairlead test bench"),
        "{bench}"
    );
    assert_eq!(item(&score, "runners")["max"], 10);

    let insights = json(&fairlead(
        &repo,
        &stubs,
        &["insights", "--suggest", "--json"],
    ));
    let suggestion = insights["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| {
            s["change"]
                .as_str()
                .unwrap()
                .starts_with("[[tests.runners]]")
        })
        .unwrap_or_else(|| panic!("{insights}"));
    assert_eq!(
        suggestion["change"],
        r#"[[tests.runners]] id = "bun", command = ["bun", "test", "--parallel", "--no-isolate", "{files}"]"#
    );
    let because = suggestion["because"].as_str().unwrap();
    assert!(
        because.contains("bun test --parallel --no-isolate 0.2")
            && because.contains("bun test 0.6"),
        "the suggestion quotes the measured times: {because}"
    );

    let config = std::fs::read_to_string(repo.join("fairlead.toml")).unwrap();
    write(
        &repo,
        "fairlead.toml",
        &config.replace(
            r#""test", "{files}""#,
            r#""test", "--parallel", "--no-isolate", "{files}""#,
        ),
    );
    let score = json(&fairlead(&repo, &stubs, &["doctor", "--score", "--json"]));
    assert_eq!(item(&score, "bench")["points"], 5, "{score}");
    let insights = json(&fairlead(
        &repo,
        &stubs,
        &["insights", "--suggest", "--json"],
    ));
    assert!(
        insights["suggestions"].as_array().unwrap().is_empty(),
        "nothing to suggest once the runner runs the mode: {insights}"
    );
    let _ = std::fs::remove_dir_all(repo.parent().unwrap());
}

#[cfg(unix)]
#[test]
fn bun_before_1_4_4_runs_nothing_and_recommends_nothing() {
    let (repo, stubs) = repo("old", "1.3.9", r#"["bun", "test", "{files}"]"#);
    let out = fairlead(&repo, &stubs, &["test", "bench"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains(
            "recommend: nothing; Bun 1.3.9 has no `bun test --parallel`, which came in Bun 1.4.4"
        ),
        "{text}"
    );
    assert!(
        !repo.parent().unwrap().join("bun.log").exists(),
        "no test ran"
    );
    let score = json(&fairlead(&repo, &stubs, &["doctor", "--score", "--json"]));
    assert_eq!(
        item(&score, "bench")["points"],
        5,
        "an old Bun has nothing better to run"
    );
    let _ = std::fs::remove_dir_all(repo.parent().unwrap());
}

#[test]
fn a_runner_that_isnt_bun_test_is_refused() {
    let (repo, stubs) = repo("vitest", "1.4.4", r#"["vitest", "run", "{files}"]"#);
    let out = fairlead(&repo, &stubs, &["test", "bench"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no runner's command is `bun test`"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = fairlead(&repo, &stubs, &["test", "bench", "--runner", "missing"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("no runner `missing`"));
    let score = json(&fairlead(&repo, &stubs, &["doctor", "--score", "--json"]));
    assert!(
        score["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["id"] != "bench"),
        "without a bun runner the runner points stay whole: {score}"
    );
    assert_eq!(item(&score, "runners")["max"], 15);
    let _ = std::fs::remove_dir_all(repo.parent().unwrap());
}
