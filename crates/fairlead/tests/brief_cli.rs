//! `fairlead brief` on a real repository: each section and its source, the
//! brief kept per session, the cap on long sections, and the once-per-session
//! note after an edit made without one.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

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
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn brief(dir: &Path, args: &[&str]) -> (i32, String) {
    let mut all = vec!["brief", "--base", "main"];
    all.extend(args);
    let out: Output = command(dir, env!("CARGO_BIN_EXE_fairlead"), &all)
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

[[checks]]
id = "typecheck"
command = ["sh", "-c", "exit 0"]

[done]
always = ["typecheck"]

[guard.size]
files = ["src/**"]
file_lines = 500
"#;

fn repo(name: &str, config: &str, importers: usize) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-brief-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
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
    for i in 0..importers {
        std::fs::write(
            dir.join(format!("src/u{i}.ts")),
            "import { a } from './a';\n",
        )
        .unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    dir
}

fn stored(dir: &Path) -> Vec<Value> {
    let briefs = dir.join(".git/fairlead/briefs");
    let mut out: Vec<Value> = std::fs::read_dir(briefs)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap())
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|b: &Value| b["id"].as_str().unwrap_or("").to_string());
    out
}

#[test]
fn every_section_is_listed_with_its_source() {
    let dir = repo("sections", CONFIG, 0);
    let (code, out) = brief(&dir, &["src/a.ts", "--session", "s1"]);
    assert_eq!(code, 0, "{out}");
    let line = |name: &str| {
        out.lines()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("no {name} line in\n{out}"))
            .to_string()
    };
    let reaches = line("reaches");
    assert!(
        reaches.contains("1 file imports them directly, 2 through the graph"),
        "{reaches}"
    );
    assert!(reaches.ends_with("graph importers, graph why"), "{reaches}");
    assert!(
        out.contains("src/b.ts") && out.contains("imports src/a.ts"),
        "{out}"
    );
    assert!(line("tests").contains("1 test file") && line("tests").ends_with("plan --files"));
    assert!(out.contains("test/b.test.ts"), "{out}");
    assert!(line("checks").contains("typecheck"), "{out}");
    let rules = line("rules");
    assert!(
        rules.contains("size") && rules.ends_with("fairlead.toml [guard.*]"),
        "{rules}"
    );
    assert!(
        line("lessons").ends_with(".fairlead/lessons (fairlead lessons list)"),
        "{out}"
    );
    assert!(line("skills").contains("(K4)"));
    assert!(line("done").contains("unit, typecheck, guard"), "{out}");
    assert_eq!(line("next"), "next     edit, then: fairlead done");

    let (_, json) = brief(&dir, &["src/a.ts", "--session", "s1", "--json"]);
    let b: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(b["reaches"]["total"], 2);
    assert_eq!(b["tests"]["items"][0]["name"], "test/b.test.ts");
    assert_eq!(
        b["rules"]["items"][0],
        json!({"table": "size", "paths": ["src/a.ts"]})
    );
    assert_eq!(b["done"]["items"][0]["name"], "unit");
    assert!(b["base"].as_str().is_some_and(|c| c.len() == 40), "{b}");
    assert_eq!(stored(&dir), vec![b], "the stored brief is the one printed");
}

#[test]
fn a_second_brief_in_a_session_adds_its_paths_and_another_session_keeps_its_own() {
    let dir = repo("session", CONFIG, 0);
    brief(&dir, &["src/a.ts", "--session", "s1"]);
    let (_, out) = brief(&dir, &["test/b.test.ts", "--session", "s1"]);
    assert!(
        out.starts_with("brief b-") && out.lines().next().unwrap().ends_with("2 paths"),
        "{out}"
    );
    brief(&dir, &["src/b.ts", "--session", "s2"]);
    let briefs = stored(&dir);
    assert_eq!(briefs.len(), 2, "{briefs:?}");
    let s1 = briefs.iter().find(|b| b["session"] == "s1").unwrap();
    assert_eq!(s1["paths"], json!(["src/a.ts", "test/b.test.ts"]));
    let s2 = briefs.iter().find(|b| b["session"] == "s2").unwrap();
    assert_eq!(s2["paths"], json!(["src/b.ts"]));

    let per_call = repo("call", &format!("{CONFIG}[brief]\nper = \"call\"\n"), 0);
    brief(&per_call, &["src/a.ts", "--session", "s1"]);
    brief(&per_call, &["src/b.ts", "--session", "s1"]);
    let briefs = stored(&per_call);
    assert_eq!(briefs.len(), 2, "each call stands alone: {briefs:?}");
    assert!(briefs
        .iter()
        .all(|b| b["paths"].as_array().unwrap().len() == 1));
}

#[test]
fn a_long_section_shows_its_first_items_and_a_count_under_forty_lines() {
    let dir = repo("cap", CONFIG, 12);
    let (_, out) = brief(&dir, &["src/a.ts"]);
    assert!(out.contains("13 files import them directly"), "{out}");
    assert!(out.contains("  … 8 more (fairlead brief --all)"), "{out}");
    assert!(
        out.lines().count() <= 40,
        "{} lines:\n{out}",
        out.lines().count()
    );
    let (_, all) = brief(&dir, &["src/a.ts", "--all"]);
    assert!(!all.contains("more (fairlead brief --all)"), "{all}");
    assert!(all.contains("src/u11.ts"), "{all}");
}

#[test]
fn a_path_that_does_not_exist_yet_is_a_new_file_with_its_rules() {
    let dir = repo("new", CONFIG, 0);
    let (code, out) = brief(&dir, &["src/new.ts"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("new      src/new.ts: not in the tree yet"),
        "{out}"
    );
    assert!(
        out.contains("0 files import them directly, 0 through the graph"),
        "{out}"
    );
    assert!(
        out.lines()
            .any(|l| l.starts_with("rules") && l.contains("size")),
        "{out}"
    );
}

/// Runs the PostToolUse hook as Claude Code would, returning what it printed.
fn nudge(dir: &Path, session: &str) -> String {
    let mut child = command(dir, env!("CARGO_BIN_EXE_fairlead"), &["guard", "nudge"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let call = json!({"session_id": session, "cwd": dir, "hook_event_name": "PostToolUse", "tool_name": "Write"});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(call.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "the hook always exits 0");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn an_edit_without_a_brief_gets_one_note_per_session_and_never_after_a_brief() {
    let dir = repo("nudge", CONFIG, 0);
    let first: Value = serde_json::from_str(nudge(&dir, "s1").trim()).unwrap();
    assert_eq!(first["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert!(first["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("fairlead brief <paths>"));
    assert_eq!(nudge(&dir, "s1"), "", "once per session");
    brief(&dir, &["src/a.ts", "--session", "s2"]);
    assert_eq!(nudge(&dir, "s2"), "", "a session with a brief isn't told");

    let off = repo("nudgeoff", &format!("{CONFIG}[brief]\nnudge = false\n"), 0);
    assert_eq!(nudge(&off, "s1"), "");
}
