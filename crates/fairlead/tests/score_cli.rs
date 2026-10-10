//! `fairlead doctor --score` on a bare repository and on one with evidence
//! for every item: the points, the fixes it names, and `--min` for CI.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

fn fairlead(dir: &Path, args: &[&str]) -> Output {
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
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-score-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(&dir, "test/a.test.ts", "import { a } from '../src/a';\n");
    dir
}

fn score(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut all = vec!["doctor", "--score", "--json"];
    all.extend_from_slice(args);
    let out = fairlead(dir, &all);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn points(s: &serde_json::Value, id: &str) -> u64 {
    s["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == id)
        .unwrap_or_else(|| panic!("no item {id}"))["points"]
        .as_u64()
        .unwrap()
}

#[test]
fn a_bare_repository_scores_zero_and_names_the_biggest_fixes_first() {
    let dir = repo("bare");
    let s = score(&dir, &[]);
    assert_eq!(
        (s["rubric"].as_u64(), s["score"].as_u64()),
        (Some(1), Some(0))
    );
    let max: u64 = s["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["max"].as_u64().unwrap())
        .sum();
    assert_eq!(max, 100, "the rubric sums to 100");
    let text = String::from_utf8_lossy(&fairlead(&dir, &["doctor", "--score"]).stdout).into_owned();
    assert!(text.starts_with("readiness 0/100 (rubric 1)"), "{text}");
    assert!(
        text.contains("next:\n  +30 run `fairlead replay fetch`"),
        "the two replay items share one fix: {text}"
    );
    let out = fairlead(&dir, &["doctor", "--score", "--min", "1"]);
    assert_eq!(out.status.code(), Some(1), "--min fails a score below it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn evidence_earns_each_item_and_config_alone_does_not() {
    let dir = repo("full");
    write(&dir, "fairlead.toml", "[[tests.runners]]\nid = \"vitest\"\nmatch = [\"test/**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n\n[stages]\nenvironments = [\"main\"]\n\n[[skills.routes]]\nskill = \".claude/skills/forms/SKILL.md\"\npaths = [\"src/**\"]\n");
    write(
        &dir,
        ".claude/skills/forms/SKILL.md",
        "---\nname: forms\ndescription: Forms.\n---\nUse the form kit.\n",
    );
    write(
        &dir,
        ".github/workflows/ci.yml",
        "jobs:\n  plan:\n    steps:\n      - run: fairlead ci plan --format github\n",
    );
    let configured = score(&dir, &[]);
    assert_eq!(
        points(&configured, "done"),
        0,
        "a configured done gate that never passed earns nothing"
    );
    assert_eq!(points(&configured, "firing"), 0);
    assert_eq!(
        points(&configured, "lessons"),
        0,
        "no lessons is not the same as none overdue"
    );
    let out = fairlead(&dir, &["hooks", "install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = fairlead(
        &dir,
        &[
            "learn",
            "--title",
            "Use the form kit",
            "--path",
            "src/**",
            "--evidence",
            "https://example.com/pr/1",
            "--source",
            "mistake",
            "--body",
            "Forms go through the kit.",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let now = fairlead_guard::events::timestamp(SystemTime::now());
    let event = |stage: &str, decision: &str| {
        format!("{{\"at\":\"{now}\",\"stage\":\"{stage}\",\"decision\":\"{decision}\",\"rules\":[],\"added\":0,\"ms\":1.0,\"fairlead\":\"test\"}}\n")
    };
    write(
        &dir,
        ".git/fairlead/events.jsonl",
        &(event("done", "pass") + &event("write", "allow")),
    );
    let report = format!("{{\"until\":\"{}\",\"judged\":40,\"min_failures\":10,\"recall\":0.97,\"median_selected\":0.4}}\n", &now[..10]);
    write(&dir, ".fairlead/replay.json", &report);
    let s = score(&dir, &[]);
    let lost: Vec<String> = s["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["points"] != i["max"])
        .map(|i| format!("{}: {}", i["id"], i["evidence"]))
        .collect();
    assert!(lost.is_empty(), "{lost:?}");
    assert_eq!(s["score"], 100);
    write(&dir, ".fairlead/replay.json", &report.replace("0.4", "1.0"));
    assert_eq!(
        points(&score(&dir, &[]), "recall"),
        0,
        "recall from plans that run everything doesn't count"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
