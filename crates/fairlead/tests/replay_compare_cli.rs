//! `fairlead replay run` against the base branch's committed config: two
//! failed runs, a config change that moves one from miss to hit and the
//! other from hit to miss, and the cases with nothing to compare.

use std::path::{Path, PathBuf};
use std::process::Command;

const RUNNER: &str = "[graph]\ncache = false\n\n[tests]\nunreached = \"warn\"\n\n[[tests.runners]]\nid = \"vitest\"\nmatch = [\"**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n\n[[replay.failures]]\nrunner = \"vitest\"\nextractor = \"vitest\"\njob = \"^test$\"\n";

fn edge(test: &str, source: &str) -> String {
    format!("\n[[graph.edges]]\nfrom = \"{test}\"\nto = [\"{source}\"]\n")
}

/// The committed config reaches `src/b.ts` through a pack file in the
/// repository, so only a pack resolved from the base's tree selects it.
fn committed() -> String {
    format!("extends = [\"packs/b.toml\"]\n\n{RUNNER}")
}

struct Fixture {
    dir: PathBuf,
    repo: PathBuf,
    data: PathBuf,
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

/// Two pull requests off `main`: run 1 changes `src/a.ts` and fails
/// `test/a.test.ts`, run 2 changes `src/b.ts` and fails `test/b.test.ts`.
/// Neither test imports anything, so only a rule selects it.
fn fixture(name: &str, config: Option<&str>) -> Fixture {
    let dir = std::env::temp_dir().join(format!("fairlead-compare-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    write(&repo, "src/a.ts", "export const a = 1;\n");
    write(&repo, "src/b.ts", "export const b = 1;\n");
    write(&repo, "test/a.test.ts", "export {};\n");
    write(&repo, "test/b.test.ts", "export {};\n");
    write(&repo, "packs/b.toml", &edge("test/b.test.ts", "src/b.ts"));
    if let Some(text) = config {
        write(&repo, "fairlead.toml", text);
    }
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let mut rows = String::new();
    for (run, file) in [(1, "a"), (2, "b")] {
        git(&repo, &["checkout", "-q", "-b", &format!("pr{run}"), &base]);
        write(&repo, &format!("src/{file}.ts"), "export const x = 2;\n");
        git(&repo, &["commit", "-q", "-am", "change"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let row = serde_json::json!({
            "repo": "example/repo", "run_id": run, "attempt": 1, "event": "pull_request",
            "workflow": "ci", "pr": run, "head_sha": head, "base_sha": base,
            "created_at": format!("2026-09-1{run}T10:00:00Z"), "conclusion": "failure",
            "jobs": [{ "name": "test", "conclusion": "failure",
                       "log": [format!(" FAIL  test/{file}.test.ts > {file}")] }]
        });
        rows.push_str(&format!("{row}\n"));
    }
    git(&repo, &["checkout", "-q", "main"]);
    let data = dir.join("data.jsonl");
    std::fs::write(&data, rows).unwrap();
    Fixture { dir, repo, data }
}

/// Replays with `config` written beside the clone, returning stdout and
/// the JSON report.
fn replay(fx: &Fixture, config: &str, extra: &[&str]) -> (String, serde_json::Value) {
    let path = fx.dir.join("trial.toml");
    std::fs::write(&path, config).unwrap();
    let json = fx.dir.join("report.json");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(["replay", "run", "--quiet", "--data"])
        .arg(&fx.data)
        .arg("--clone")
        .arg(&fx.repo)
        .arg("--config")
        .arg(&path)
        .arg("--json-out")
        .arg(&json)
        .args(extra)
        .current_dir(&fx.dir)
        .env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = serde_json::from_str(&std::fs::read_to_string(json).unwrap()).unwrap();
    (String::from_utf8_lossy(&out.stdout).into_owned(), report)
}

fn trial() -> String {
    format!("{RUNNER}{}", edge("test/a.test.ts", "src/a.ts"))
}

#[test]
fn a_config_change_lists_each_failure_it_moves_between_hit_and_miss() {
    let fx = fixture("moves", Some(&committed()));
    let (text, json) = replay(&fx, &trial(), &[]);
    assert!(
        text.contains("compared with fairlead.toml at HEAD ("),
        "{text}"
    );
    assert!(
        text.contains("moved: 1 hit to miss, 1 miss to hit"),
        "{text}"
    );
    assert!(
        text.contains("miss -> hit  run 1 attempt 1 (PR 1)  test/a.test.ts  [test]"),
        "{text}"
    );
    assert!(
        text.contains("hit -> miss  run 2 attempt 1 (PR 2)  test/b.test.ts  [test]"),
        "{text}"
    );
    assert!(
        text.contains("base: not in the plan\n        this: selected: "),
        "{text}"
    );
    let c = &json["comparison"];
    assert_eq!(c["status"], "compared");
    assert_eq!(c["base"]["recall"], 0.5);
    assert_eq!(c["this"]["recall"], 0.5);
    let moved = c["moved"].as_array().unwrap();
    assert_eq!(moved.len(), 2, "{c}");
    assert_eq!(moved[0]["target"], "test/a.test.ts");
    assert_eq!(moved[0]["from"], "miss");
    assert_eq!(moved[1]["target"], "test/b.test.ts");
    assert_eq!(moved[1]["base"]["outcome"], "hit");
    assert_eq!(moved[1]["this"]["why"], "not in the plan");
    assert_eq!(
        c["base_commit"].as_str().unwrap(),
        git(&fx.repo, &["rev-parse", "main"])
    );
    assert_eq!(json["recall"], 0.5, "the report itself is this config's");
    assert_eq!(json["misses"][0]["target"], "test/b.test.ts");
}

#[test]
fn a_config_that_resolves_alike_prints_no_comparison() {
    let fx = fixture("same", Some(&committed()));
    // The pack's rule written inline, and reformatted: the bytes differ,
    // the loaded config doesn't.
    let inline = format!(
        "# a comment\n{RUNNER}{}",
        edge("test/b.test.ts", "src/b.ts")
    );
    let (text, json) = replay(&fx, &inline, &[]);
    assert!(!text.contains("compared with"), "{text}");
    assert!(
        text.contains("base config: the config plans alike to fairlead.toml at HEAD ("),
        "{text}"
    );
    assert_eq!(json["comparison"]["status"], "same");
    assert!(json["comparison"].get("base").is_none());
}

#[test]
fn with_no_committed_config_a_note_says_so_and_compare_with_names_one() {
    let fx = fixture("none", None);
    let (text, json) = replay(&fx, &trial(), &[]);
    assert!(
        text.contains("base config: no fairlead.toml, .yaml or .yml is committed at HEAD ("),
        "{text}"
    );
    assert_eq!(json["comparison"]["status"], "no_base");
    let other = fx.dir.join("other.toml");
    std::fs::write(
        &other,
        format!("{RUNNER}{}", edge("test/b.test.ts", "src/b.ts")),
    )
    .unwrap();
    let (text, json) = replay(&fx, &trial(), &["--compare-with", other.to_str().unwrap()]);
    assert!(
        text.contains(&format!("compared with {}:", other.display())),
        "{text}"
    );
    assert_eq!(json["comparison"]["moved"].as_array().unwrap().len(), 2);
}

#[test]
fn a_committed_config_with_problems_is_noted_not_replayed() {
    let broken = format!(
        "{RUNNER}\n[[replay.quarantine]]\npath = \"test/a.test.ts\"\njob = \"^test$\"\nreason = \"flaky\"\nuntil = \"2026/12/31\"\n"
    );
    let fx = fixture("broken", Some(&broken));
    let (text, json) = replay(&fx, &trial(), &[]);
    assert!(text.contains("has problems (replay.quarantine"), "{text}");
    assert_eq!(json["comparison"]["status"], "unusable");
}

#[test]
fn no_compare_leaves_the_report_as_it_was() {
    let fx = fixture("off", Some(&committed()));
    let (text, json) = replay(&fx, &trial(), &["--no-compare"]);
    assert!(!text.contains("base config"), "{text}");
    assert!(!text.contains("compared with"), "{text}");
    assert!(json.get("comparison").is_none());
    assert_eq!(json["recall"], 0.5);
}
