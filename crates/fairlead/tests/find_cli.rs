//! `fairlead find` on a real repository: a lesson is found by a word in its
//! body, a skill by its description, a docs heading by its text and a PHP
//! class by its name; hits stop at 20 unless `--limit` says otherwise; the
//! JSON names each hit's kind, name, path, score and snippet; and a hit
//! near the session's brief outranks an equal one that isn't.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    // A clean environment, so a developer's FAIRLEAD_*, CI, git or session settings can't leak in.
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let out = command(dir, "git", args).output().unwrap();
    assert!(out.status.success(), "git {args:?}");
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

const CONFIG: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]

[[skills.routes]]
skill = ".claude/skills/forms/SKILL.md"
paths = ["src/**"]
"#;

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn lesson(dir: &Path, id: &str, title: &str, paths: &str, body: &str) {
    write(
        dir,
        &format!(".fairlead/lessons/{id}.md"),
        &format!("---\nid: {id}\ntitle: \"{title}\"\npaths: [\"{paths}\"]\nadded: 2026-01-01\nevidence: [\"https://example.com/1\"]\nsource: mistake\n---\n{body}\n"),
    );
}

/// test/b.test.ts imports src/b.ts, which imports src/a.ts; other/z.ts is
/// on its own.
fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-find-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", CONFIG);
    write(&dir, "package.json", "{\"name\":\"x\"}\n");
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(
        &dir,
        "src/b.ts",
        "import { a } from './a';\nexport const b = a;\n",
    );
    write(&dir, "other/z.ts", "export const z = 1;\n");
    write(&dir, "test/b.test.ts", "import { b } from '../src/b';\n");
    write(
        &dir,
        "app/Models/Invoice.php",
        "<?php\nnamespace App\\Models;\n\nclass Invoice {}\n",
    );
    lesson(
        &dir,
        "money-rounding",
        "Round once",
        "src/**",
        "Totals are rounded at the ledger boundary, never per line.",
    );
    write(
        &dir,
        ".claude/skills/forms/SKILL.md",
        "---\nname: web-forms\ndescription: How every form validates its fields before submit.\n---\nBody.\n",
    );
    write(
        &dir,
        "docs/deploy.md",
        "# Deploying\n\nBlue and green.\n\n## Rolling back a release\n\nPick the last good tag.\n",
    );
    write(
        &dir,
        "node_modules/pkg/README.md",
        "# Rolling back a release\n",
    );
    dir
}

fn commit(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let mut all = vec!["find", "--json"];
    all.extend(args);
    let (code, out) = fairlead(dir, &all);
    assert_eq!(code, 0, "{out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"))
}

fn first(answer: &Value) -> (&str, &str, &str) {
    let hit = &answer["hits"][0];
    (
        hit["kind"].as_str().unwrap(),
        hit["name"].as_str().unwrap(),
        hit["path"].as_str().unwrap(),
    )
}

#[test]
fn each_kind_is_found_by_its_own_words() {
    let dir = repo("kinds");
    commit(&dir);
    assert_eq!(
        first(&json(&dir, &["ledger", "boundary"])),
        (
            "lesson",
            "Round once",
            ".fairlead/lessons/money-rounding.md"
        )
    );
    assert_eq!(
        first(&json(&dir, &["validates", "fields"])),
        ("skill", "web-forms", ".claude/skills/forms/SKILL.md")
    );
    let rollback = json(&dir, &["rolling", "back"]);
    assert_eq!(
        first(&rollback),
        (
            "doc",
            "Rolling back a release",
            "docs/deploy.md#rolling-back-a-release"
        )
    );
    assert_eq!(rollback["hits"].as_array().unwrap().len(), 1, "{rollback}");
    assert_eq!(
        first(&json(&dir, &["invoice"])),
        ("symbol", "App\\Models\\Invoice", "app/Models/Invoice.php")
    );
    assert_eq!(
        first(&json(&dir, &["--symbol", "invoice"])).1,
        "App\\Models\\Invoice"
    );
    assert!(json(&dir, &["--symbol", "Invo"])["hits"]
        .as_array()
        .unwrap()
        .is_empty());

    let (code, out) = fairlead(&dir, &["find", "ledger"]);
    assert_eq!(code, 0, "{out}");
    let line = out.lines().next().unwrap();
    let cells: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(cells[0], "lesson", "{out}");
    assert!(line.contains("Round once  .fairlead/lessons/money-rounding.md  "));
    assert!(cells.last().unwrap().parse::<f64>().unwrap() > 0.0, "{out}");
    let (code, out) = fairlead(&dir, &["find", "zzzz"]);
    assert_eq!((code, out.as_str()), (0, "nothing matches `zzzz`\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hits_stop_at_twenty_unless_the_limit_says_otherwise_and_json_names_each_field() {
    let dir = repo("cap");
    let headings: String = (0..30)
        .map(|i| format!("## Widget step {i}\n\nThe widget does part {i}.\n\n"))
        .collect();
    write(&dir, "docs/widgets.md", &headings);
    commit(&dir);
    let answer = json(&dir, &["widget"]);
    let hits = answer["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 20);
    assert_eq!(answer["query"], "widget");
    assert_eq!(answer["brief"], Value::Null);
    for hit in hits {
        let keys: Vec<&String> = hit.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["kind", "name", "path", "score", "snippet"], "{hit}");
        assert_eq!(hit["kind"], "doc");
        assert!(hit["score"].as_f64().unwrap() > 0.0);
    }
    assert!(hits[0]["snippet"]
        .as_str()
        .unwrap()
        .starts_with("The widget does part"));
    let scores: Vec<f64> = hits.iter().map(|h| h["score"].as_f64().unwrap()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
    assert_eq!(
        json(&dir, &["widget", "--limit", "50"])["hits"]
            .as_array()
            .unwrap()
            .len(),
        30
    );
    let (code, out) = fairlead(&dir, &["find", "widget", "--limit", "51"]);
    assert_eq!(code, 2, "{out}");
    let (code, out) = fairlead(&dir, &["find", "widget"]);
    assert_eq!((code, out.lines().count()), (0, 20), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_lesson_near_the_sessions_brief_outranks_an_equal_one_that_isnt() {
    let dir = repo("near");
    // The same words; only the scope differs, and "Alpha" sorts first.
    lesson(
        &dir,
        "alpha-far",
        "Alpha note",
        "other/**",
        "Retries back off.",
    );
    lesson(
        &dir,
        "zulu-near",
        "Zulu note",
        "src/a.ts",
        "Retries back off.",
    );
    commit(&dir);
    let names = |answer: &Value| -> Vec<String> {
        answer["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        names(&json(&dir, &["retries"])),
        ["Alpha note", "Zulu note"]
    );
    let (code, out) = fairlead(
        &dir,
        &["brief", "--base", "main", "--session", "s1", "src/b.ts"],
    );
    assert_eq!(code, 0, "{out}");
    let boosted = json(&dir, &["retries", "--session", "s1"]);
    assert_eq!(names(&boosted), ["Zulu note", "Alpha note"]);
    assert!(boosted["brief"].as_str().unwrap().starts_with("b-"));
    let ratio = boosted["hits"][0]["score"].as_f64().unwrap()
        / boosted["hits"][1]["score"].as_f64().unwrap();
    assert!((ratio - 1.5).abs() < 0.01, "one hop away is 1.5x: {ratio}");
    // Another session has no brief, so nothing is boosted.
    assert_eq!(
        names(&json(&dir, &["retries", "--session", "s2"])),
        ["Alpha note", "Zulu note"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
