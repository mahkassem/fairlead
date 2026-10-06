//! `fairlead import lessons` on a real repository: a dry run writes nothing,
//! `--write` makes one lesson per `## ` heading and is idempotent, a heading
//! with a secret is refused without stopping the rest, the result passes
//! `lessons check`, and the brief offers an imported lesson by its paths.

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
        .env("GIT_AUTHOR_DATE", "2026-01-15T12:00:00Z")
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
"#;

fn long_section() -> String {
    let lines: Vec<String> = (1..=15)
        .map(|i| format!("Step {i} of the story about `src/b.ts`."))
        .collect();
    lines.join("\n")
}

/// Eight headings: codes and titles, a secret, a long one, one with no
/// path, a repeated one, and a slug naming a file that doesn't exist.
fn document() -> String {
    format!(
        "# Lessons\n\nWhat we learned, one heading each.\n\n\
## AB12\n\nTotals are computed once, at the end. They live in `src/a.ts`; see https://example.com/pr/12.\n\n\
## Keep imports one way\n\nCode under `src` never imports a test such as `test/b.test.ts`.\n\n\
## Rotate keys\n\nNever paste one: AKIAABCDEFGHIJKLMNOP is what not to do.\n\n\
## Long story\n\n{}\n\n\
## House style\n\nWrite short sentences. `npm test` is a command, not a path.\n\n\
## House style\n\nA second note under the same heading.\n\n\
## keep-diffs-small\n\nSmall diffs are reviewed well. `src/gone.ts` is not here.\n\n\
## Totals in `src/b.ts`\n\nThe total is computed once.\n",
        long_section()
    )
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["src", "test", "docs"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join("package.json"), "{\"name\":\"x\"}\n").unwrap();
    std::fs::write(dir.join("src/a.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(
        dir.join("src/b.ts"),
        "import { a } from './a';\nexport const b = a;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("test/b.test.ts"),
        "import { b } from '../src/b';\n",
    )
    .unwrap();
    std::fs::write(dir.join("docs/LESSONS.md"), document()).unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn lesson_files(dir: &Path) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir.join(".fairlead/lessons"))
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    files.sort();
    files
}

#[test]
fn a_lessons_document_becomes_one_lesson_per_heading_and_reimports_unchanged() {
    let dir = repo("doc");
    let (code, out) = fairlead(&dir, &["import", "lessons", "docs/LESSONS.md"]);
    assert_eq!(code, 1, "a skipped heading exits 1: {out}");
    assert!(
        out.contains("import: docs/LESSONS.md: 8 headings → 7 lessons, 4 with paths, 3 found only by search, 1 skipped (a secret or personal data)"),
        "{out}"
    );
    assert!(
        out.contains("preamble: 2 lines before the first `## ` heading, skipped"),
        "{out}"
    );
    assert!(
        out.contains("ab12  [src/a.ts]  Totals are computed once, at the end."),
        "{out}"
    );
    assert!(
        out.contains("keep-imports-one-way  [src/**, test/b.test.ts]  Keep imports one way"),
        "{out}"
    );
    assert!(
        out.contains("house-style-2  [search]  House style"),
        "{out}"
    );
    assert!(
        out.contains("`house-style` is taken by an earlier heading"),
        "{out}"
    );
    assert!(
        out.contains("keep-diffs-small  [search]  Small diffs are reviewed well."),
        "{out}"
    );
    assert!(
        out.contains("no path in the text; found only by search"),
        "{out}"
    );
    assert!(
        out.contains(
            "skipped  ## Rotate keys (line 13): docs/LESSONS.md:15: looks like a cloud access key"
        ),
        "{out}"
    );
    assert!(
        !out.contains("AKIAABCDEFGHIJKLMNOP"),
        "never echoes it: {out}"
    );
    assert!(!dir.join(".fairlead").exists(), "a dry run writes nothing");

    let (code, out) = fairlead(&dir, &["import", "lessons", "docs/LESSONS.md", "--write"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("wrote 7 lessons to .fairlead/lessons"),
        "{out}"
    );
    let files = lesson_files(&dir);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "ab12.md",
            "house-style-2.md",
            "house-style.md",
            "keep-diffs-small.md",
            "keep-imports-one-way.md",
            "long-story.md",
            "totals-in-src-b-ts.md",
        ]
    );
    let ab12 = &files[0].1;
    assert!(
        ab12.starts_with("---\nid: ab12\ntitle: \"Totals are computed once, at the end.\"\npaths: [\"src/a.ts\"]\nadded: 2026-01-15\nevidence: [\"docs/LESSONS.md#ab12\", \"https://example.com/pr/12\"]\nsource: imported\n---\n"),
        "{ab12}"
    );
    let long = &files[5].1;
    let body: Vec<&str> = long.split("\n---\n").nth(1).unwrap().lines().collect();
    assert_eq!(body.len(), 12, "{long}");
    assert_eq!(body[10], "Step 11 of the story about `src/b.ts`.");
    assert_eq!(body[11], "Full text: docs/LESSONS.md#long-story");
    assert_eq!(
        std::fs::read_to_string(dir.join("docs/LESSONS.md")).unwrap(),
        document(),
        "the document is never written"
    );

    let (_, out) = fairlead(&dir, &["import", "lessons", "docs/LESSONS.md", "--write"]);
    assert!(
        out.contains(
            "wrote 0 lessons to .fairlead/lessons; 7 already there and the same, 0 refused"
        ),
        "{out}"
    );
    assert_eq!(lesson_files(&dir), files, "a second import changes nothing");

    let (code, out) = fairlead(&dir, &["lessons", "check"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("lessons: 7 fine, 0 that can't be offered"),
        "{out}"
    );

    let (code, out) = fairlead(
        &dir,
        &[
            "brief",
            "--base",
            "main",
            "--session",
            "s1",
            "--json",
            "src/a.ts",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let brief: Value = serde_json::from_str(&out).unwrap();
    let items = brief["lessons"]["items"].as_array().unwrap();
    let ab12 = items
        .iter()
        .find(|i| i["name"] == "ab12")
        .expect("ab12 is offered");
    assert!(
        ab12["why"].as_str().unwrap().contains("named: src/a.ts"),
        "{items:?}"
    );
}

#[test]
fn a_different_file_under_the_same_name_is_refused_not_overwritten() {
    let dir = repo("refuse");
    std::fs::create_dir_all(dir.join(".fairlead/lessons")).unwrap();
    let mine = "---\nid: house-style\ntitle: \"Mine\"\nalways: true\nadded: 2026-01-01\nevidence: [\"x\"]\nsource: mistake\n---\nKeep it.\n";
    std::fs::write(dir.join(".fairlead/lessons/house-style.md"), mine).unwrap();
    let (code, out) = fairlead(
        &dir,
        &["import", "lessons", "docs/LESSONS.md", "--write", "--json"],
    );
    assert_eq!(code, 1, "{out}");
    let import: Value = serde_json::from_str(&out).unwrap();
    let lessons = import["lessons"].as_array().unwrap();
    let status = |id: &str| {
        lessons
            .iter()
            .find(|l| l["lesson"]["id"] == id)
            .map(|l| l["status"].as_str().unwrap().to_string())
    };
    assert_eq!(status("house-style").as_deref(), Some("refused"));
    assert_eq!(status("ab12").as_deref(), Some("written"));
    assert_eq!(import["skipped"].as_array().unwrap().len(), 1);
    assert_eq!(
        std::fs::read_to_string(dir.join(".fairlead/lessons/house-style.md")).unwrap(),
        mine
    );
}

#[test]
fn six_hundred_and_fifty_headings_are_each_a_lesson_or_a_reason() {
    let dir = repo("many");
    let mut doc = String::from("Intro.\n\n");
    for i in 0..650 {
        let heading = match i % 4 {
            0 => format!("C{i}"),
            1 => format!("Rule number {}", i % 40),
            2 => format!("rule-{i}"),
            _ => format!("About `src/a.ts` part {i}"),
        };
        let body = match i % 50 {
            7 => "A card 4111 1111 1111 1111 slipped in.".to_string(),
            9 => String::new(),
            _ => format!("Lesson {i} holds. See `src/b.ts` and https://example.com/{i}."),
        };
        doc.push_str(&format!("## {heading}\n\n{body}\n\n"));
    }
    std::fs::write(dir.join("docs/MANY.md"), &doc).unwrap();
    let (_, out) = fairlead(&dir, &["import", "lessons", "docs/MANY.md", "--json"]);
    let import: Value = serde_json::from_str(&out).unwrap();
    let made = import["lessons"].as_array().unwrap().len();
    let skipped = import["skipped"].as_array().unwrap().len();
    assert_eq!(import["headings"], 650);
    assert_eq!(made + skipped, 650, "every heading is accounted for");
    assert_eq!(skipped, 26, "the cards and the empty ones");
    let mut ids: Vec<&str> = import["lessons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["lesson"]["id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), made, "ids are unique");
}
