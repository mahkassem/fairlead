//! Lessons on a real repository: `learn` writes one only with a scope,
//! evidence and a source; the brief offers them named, used, always and as
//! the fallback, capped; a bad file is named, not offered; and `[memory]`
//! never moves a plan id.

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
"#;

/// test/b.test.ts imports src/b.ts, which imports src/a.ts.
fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-lessons-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
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
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn learn(dir: &Path, extra: &[&str]) -> (i32, String) {
    let mut args = vec!["learn", "--evidence", "https://example.com/pr/7"];
    if !extra.contains(&"--body") {
        args.extend(["--body", "Why it matters, in a line."]);
    }
    args.extend(extra);
    fairlead(dir, &args)
}

fn lesson_names(dir: &Path, path: &str) -> Vec<(String, String)> {
    // Each call its own session, since a session's briefs add up their paths.
    let session = format!("s-{path}");
    let (code, out) = fairlead(
        dir,
        &[
            "brief",
            "--base",
            "main",
            "--session",
            &session,
            "--json",
            path,
        ],
    );
    assert_eq!(code, 0, "{out}");
    let brief: Value = serde_json::from_str(&out).unwrap();
    brief["lessons"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap().to_string(),
                i["why"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn learn_needs_a_scope_evidence_and_a_source_and_refuses_a_secret() {
    let dir = repo("learn", CONFIG);
    let (code, out) = learn(&dir, &["--title", "Keep a stable", "--source", "person"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("a scope is needed") && out.contains("confirmed_by"),
        "{out}"
    );

    let (code, out) = learn(
        &dir,
        &[
            "--title",
            "Mail them",
            "--path",
            "src/**",
            "--source",
            "mistake",
            "--body",
            "write to a.person@company.io",
        ],
    );
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("email address") && !out.contains("a.person@"),
        "{out}"
    );

    let (code, out) = learn(
        &dir,
        &[
            "--title",
            "Settle at sign-off",
            "--path",
            "src/a.ts",
            "--source",
            "person",
            "--confirmed-by",
            "a-reviewer",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let text =
        std::fs::read_to_string(dir.join(".fairlead/lessons/settle-at-sign-off.md")).unwrap();
    assert!(
        text.contains("source: person\nconfirmed_by: \"a-reviewer\""),
        "{text}"
    );
    assert!(text.contains("review_by: "), "{text}");

    let (code, out) = learn(
        &dir,
        &[
            "--title",
            "Settle at sign-off",
            "--path",
            "src/a.ts",
            "--source",
            "mistake",
        ],
    );
    assert_eq!(code, 2, "a second file with the same id is refused: {out}");
    assert_eq!(fairlead(&dir, &["lessons", "check"]).0, 0);
}

#[test]
fn ask_prints_the_lesson_instead_of_writing_it() {
    let dir = repo("ask", &format!("{CONFIG}\n[memory]\nlearn = \"ask\"\n"));
    let (code, out) = learn(
        &dir,
        &["--title", "Ask first", "--always", "--source", "mistake"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("id: ask-first") && out.contains("nothing was written"),
        "{out}"
    );
    assert!(!dir.join(".fairlead").exists());
}

#[test]
fn the_brief_offers_named_then_used_then_always_and_never_unrelated() {
    let dir = repo("route", CONFIG);
    for (title, scope) in [
        ("About a", "src/a.ts"),
        ("About b", "src/b.ts"),
        ("About tests", "test/**"),
        ("Elsewhere", "docs/**"),
    ] {
        let (code, out) = learn(
            &dir,
            &["--title", title, "--path", scope, "--source", "mistake"],
        );
        assert_eq!(code, 0, "{out}");
    }
    let (code, out) = learn(
        &dir,
        &["--title", "House style", "--always", "--source", "mistake"],
    );
    assert_eq!(code, 0, "{out}");

    let picked = lesson_names(&dir, "src/b.ts");
    let names: Vec<&str> = picked.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["about-b", "about-a", "house-style"], "{picked:?}");
    assert!(picked[0].1.contains("named: src/b.ts"), "{picked:?}");
    assert!(
        picked[1].1.contains("used: src/b.ts imports src/a.ts"),
        "{picked:?}"
    );
    assert!(picked[2].1.contains("always"), "{picked:?}");

    // Reach goes forward only: a change to a.ts never needs b's lesson.
    let names: Vec<String> = lesson_names(&dir, "src/a.ts")
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["about-a", "house-style"]);

    // A change the plan can't follow offers everything, marked as the fallback.
    let picked = lesson_names(&dir, "package.json");
    assert_eq!(picked.len(), 5, "{picked:?}");
    assert!(
        picked
            .iter()
            .filter(|(n, _)| n != "house-style")
            .all(|(_, w)| w.contains("fallback")),
        "{picked:?}"
    );
}

#[test]
fn the_brief_caps_lessons_and_names_a_bad_file() {
    let dir = repo("cap", CONFIG);
    for i in 0..8 {
        let title = format!("Rule {i}");
        let (code, out) = learn(
            &dir,
            &["--title", &title, "--path", "src/**", "--source", "mistake"],
        );
        assert_eq!(code, 0, "{out}");
    }
    std::fs::write(
        dir.join(".fairlead/lessons/broken.md"),
        "---\nid: broken\ntitle: \"No scope\"\nadded: 2026-01-01\nevidence: [\"x\"]\nsource: mistake\n---\nbody\n",
    )
    .unwrap();
    let (code, out) = fairlead(&dir, &["brief", "--base", "main", "src/a.ts"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("lessons  8 lessons"), "{out}");
    assert!(out.contains("… 3 more (fairlead brief --all)"), "{out}");
    assert!(
        out.contains("warning  [bad-lesson] .fairlead/lessons/broken.md: a scope is needed"),
        "{out}"
    );
    assert!(out.lines().count() < 40, "{out}");
    let (code, out) = fairlead(&dir, &["lessons", "check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("8 fine, 1 that can't be offered"), "{out}");
}

#[test]
fn a_lesson_past_review_is_still_offered_and_marked_due() {
    let dir = repo("due", CONFIG);
    std::fs::create_dir_all(dir.join(".fairlead/lessons")).unwrap();
    std::fs::write(
        dir.join(".fairlead/lessons/old.md"),
        "---\nid: old\ntitle: \"Old but true\"\npaths: [\"src/**\"]\nadded: 2020-01-01\nreview_by: 2020-04-01\nevidence: [\"https://example.com/1\"]\ncheck: \"guard:size\"\nsource: mistake\n---\nStill holds.\n",
    )
    .unwrap();
    let picked = lesson_names(&dir, "src/a.ts");
    assert!(picked[0].1.contains("due for review"), "{picked:?}");
    let (_, out) = fairlead(&dir, &["lessons", "review"]);
    assert!(out.contains("old (due)"), "{out}");
}

#[test]
fn memory_settings_never_move_a_plan_id() {
    let plan_id = |config: &str| {
        let dir = repo(&format!("digest-{}", config.len()), config);
        std::fs::write(dir.join("src/a.ts"), "export const a = 2;\n").unwrap();
        let (code, out) = fairlead(&dir, &["plan", "--base", "main", "--json"]);
        assert_eq!(code, 0, "{out}");
        let plan: Value = serde_json::from_str(&out).unwrap();
        plan["config_digest"].as_str().unwrap().to_string()
    };
    assert_eq!(
        plan_id(CONFIG),
        plan_id(&format!("{CONFIG}\n[memory]\ncap = 3\nreview_days = 30\n"))
    );
}
