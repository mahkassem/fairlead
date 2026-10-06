//! `fairlead skills report` on a real repository: the four counts per skill
//! from a written event log, a miss and what can't undo it, the window, an
//! agent whose use isn't seen, lessons by offers only, text and JSON
//! agreeing, the empty log, and a report over events the brief and the hooks
//! really recorded.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    assert!(
        command(dir, "git", args).output().unwrap().status.success(),
        "git {args:?}"
    );
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

fn hook(dir: &Path, stage: &str, call: Value) -> String {
    let mut child = command(dir, env!("CARGO_BIN_EXE_fairlead"), &["guard", stage])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(call.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "a hook always exits 0");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-report-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/forms")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    std::fs::write(dir.join("src/money.ts"), "export const m = 1;\n").unwrap();
    std::fs::write(
        dir.join("test/form.test.ts"),
        "import { f } from '../src/forms/form';\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/forms/form.ts"),
        "import { m } from '../money';\nexport const f = m;\n",
    )
    .unwrap();
    for (n, d) in [("forms", "How forms validate."), ("money", "Amounts.")] {
        let d2 = dir.join(format!(".claude/skills/{n}"));
        std::fs::create_dir_all(&d2).unwrap();
        let text = format!("---\nname: {n}\ndescription: {d}\n---\nHow to.\n");
        std::fs::write(d2.join("SKILL.md"), text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn event(
    day: &str,
    stage: &str,
    session: Option<&str>,
    tool: Option<&str>,
    items: &[&str],
) -> Value {
    let mut e = json!({"at": format!("{day}T10:00:00.000Z"), "stage": stage, "decision": "brief", "rules": [], "added": 0, "ms": 0.0, "fairlead": "0", "items": items});
    if let Some(s) = session {
        e["session"] = json!(s);
    }
    if let Some(t) = tool {
        e["tool"] = json!(t);
    }
    e
}

/// s1 is offered forms, money and a lesson and uses forms; s2 uses money
/// before an offer of it; s3 is offered forms and uses nothing; s4 is a
/// Codex session, offered forms; one brief had no session.
fn fixture(dir: &Path) {
    let lines = [
        event(
            "2026-10-01",
            "offer",
            Some("s1"),
            None,
            &["lesson:settle", "skill:forms", "skill:money"],
        ),
        event("2026-10-01", "write", None, Some("Edit"), &[]),
        event(
            "2026-10-01",
            "use",
            Some("s1"),
            Some("Skill"),
            &["skill:forms"],
        ),
        event(
            "2026-10-02",
            "use",
            Some("s2"),
            Some("Skill"),
            &["skill:money"],
        ),
        event(
            "2026-10-02",
            "offer",
            Some("s2"),
            Some("Edit"),
            &["skill:money"],
        ),
        event("2026-10-02", "offer", Some("s2"), None, &["lesson:settle"]),
        event("2026-10-03", "offer", Some("s3"), None, &["skill:forms"]),
        event("2026-10-04", "write", Some("s4"), Some("apply_patch"), &[]),
        event(
            "2026-10-04",
            "offer",
            Some("s4"),
            None,
            &["lesson:tone", "skill:forms"],
        ),
        event("2026-10-04", "offer", None, None, &["skill:forms"]),
    ];
    let log: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    std::fs::create_dir_all(dir.join(".git/fairlead")).unwrap();
    std::fs::write(dir.join(".git/fairlead/events.jsonl"), log).unwrap();
}

fn json_report(dir: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["skills", "report", "--json"];
    args.extend_from_slice(extra);
    let (code, out) = fairlead(dir, &args);
    assert_eq!(code, 0, "{out}");
    serde_json::from_str(&out).unwrap()
}

/// Each skill row as (name, offered, used, missed, unused).
fn rows(report: &Value) -> Vec<(String, u64, u64, u64, u64)> {
    report["skills"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let n = |k: &str| r[k].as_u64().unwrap();
            (
                r["name"].as_str().unwrap().into(),
                n("offered"),
                n("used"),
                n("missed"),
                n("unused"),
            )
        })
        .collect()
}

#[test]
fn each_of_the_four_counts_is_right_on_a_written_log() {
    let dir = repo("counts", "");
    fixture(&dir);
    let r = json_report(&dir, &[]);
    assert_eq!(
        rows(&r),
        [("forms".into(), 3, 1, 0, 1), ("money".into(), 2, 1, 1, 1)],
        "sorted by offered, most first"
    );
    assert_eq!(r["sessions"], 4);
    assert_eq!(r["offers_without_session"], 1);
    assert_eq!(r["skills"]["sessions_offered"], 3, "s4 is unmeasured");
    assert_eq!(r["skills"]["sessions_hit"], 1);
    assert_eq!(r["skills"]["misses"], 1);
    assert!((r["skills"]["hit_rate"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-9);
}

#[test]
fn a_use_before_any_offer_is_a_miss_and_a_later_offer_does_not_make_it_a_hit() {
    let dir = repo("miss", "");
    fixture(&dir);
    let r = json_report(&dir, &["--session", "s2"]);
    assert_eq!(rows(&r), [("money".into(), 1, 1, 1, 0)]);
    assert_eq!(r["skills"]["sessions_offered"], 1);
    assert_eq!(
        r["skills"]["sessions_hit"], 0,
        "the offer came after the use"
    );
    assert_eq!(r["skills"]["hit_rate"], 0.0);
}

#[test]
fn since_and_session_narrow_the_window() {
    let dir = repo("window", "");
    fixture(&dir);
    let r = json_report(&dir, &["--since", "2026-10-03"]);
    assert_eq!(r["sessions"], 2);
    assert_eq!(rows(&r), [("forms".into(), 2, 0, 0, 1)]);
    assert_eq!(r["since"], "2026-10-03");
    let r = json_report(&dir, &["--session", "s1"]);
    assert_eq!(
        rows(&r),
        [("forms".into(), 1, 1, 0, 0), ("money".into(), 1, 0, 0, 1)]
    );
    assert_eq!(
        r["offers_without_session"], 0,
        "a session filter can't place them"
    );
    let (code, out) = fairlead(&dir, &["skills", "report", "--since", "1 October"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("YYYY-MM-DD"), "{out}");
}

#[test]
fn a_codex_session_is_unmeasured_never_unused() {
    let dir = repo("codex", "");
    fixture(&dir);
    let r = json_report(&dir, &[]);
    assert_eq!(r["unmeasured"]["sessions"], json!(["s4"]));
    assert_eq!(
        r["unmeasured"]["agents"],
        json!([]),
        "no Codex hooks installed yet"
    );
    let (code, out) = fairlead(&dir, &["hooks", "install", "--codex"]);
    assert_eq!(code, 0, "{out}");
    let r = json_report(&dir, &[]);
    assert_eq!(r["unmeasured"]["agents"], json!(["codex"]));
    let (_, text) = fairlead(&dir, &["skills", "report"]);
    assert!(
        text.contains("unmeasured: codex, whose hooks don't see a skill load; 1 session not counted as unused"),
        "{text}"
    );
}

#[test]
fn lessons_report_offers_only_and_say_their_use_is_not_measured() {
    let dir = repo("lessons", "");
    fixture(&dir);
    let r = json_report(&dir, &[]);
    assert_eq!(
        r["lessons"],
        json!({"sessions_offered": 3, "use_measured": false, "items": [{"name": "settle", "offered": 2}, {"name": "tone", "offered": 1}]})
    );
    let (_, text) = fairlead(&dir, &["skills", "report"]);
    assert!(
        text.contains("lessons: offered in 3 sessions; use of lessons isn't measured: there's no event for it"),
        "{text}"
    );
}

#[test]
fn the_text_gives_the_same_numbers_as_json() {
    let dir = repo("same", "");
    fixture(&dir);
    let (code, text) = fairlead(&dir, &["skills", "report"]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains(
            "skills: offered in 3 sessions, an offered skill used in 1 (hit rate 33%); 1 miss"
        ),
        "{text}"
    );
    assert!(
        text.contains("1 offer with no session, not counted"),
        "{text}"
    );
    let table: Vec<(String, u64, u64, u64, u64)> = text
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("skill "))
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let n = |i: usize| w[i].parse().unwrap();
            (w[0].to_string(), n(1), n(2), n(3), n(4))
        })
        .collect();
    assert_eq!(table, rows(&json_report(&dir, &[])));
    assert!(text.lines().count() <= 40, "{text}");
}

#[test]
fn the_tables_show_the_first_ten_unless_all() {
    let dir = repo("all", "");
    let names: Vec<String> = (0..12).map(|i| format!("skill:s{i:02}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    std::fs::create_dir_all(dir.join(".git/fairlead")).unwrap();
    let line = event("2026-10-01", "offer", Some("a"), None, &names).to_string() + "\n";
    std::fs::write(dir.join(".git/fairlead/events.jsonl"), line).unwrap();
    let (_, text) = fairlead(&dir, &["skills", "report"]);
    assert!(text.contains("s09") && !text.contains("s10"), "{text}");
    assert!(text.contains("… 2 more (--all)"), "{text}");
    let (_, all) = fairlead(&dir, &["skills", "report", "--all"]);
    assert!(
        all.contains("s11") && !all.contains("more (--all)"),
        "{all}"
    );
}

#[test]
fn no_events_is_one_line_saying_how_they_get_recorded() {
    let dir = repo("empty", "");
    let (code, out) = fairlead(&dir, &["skills", "report"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.contains("no offers or uses recorded yet"), "{out}");
    assert!(
        out.contains("`fairlead brief`") && out.contains("PostToolUse"),
        "{out}"
    );
    let r = json_report(&dir, &[]);
    assert_eq!(
        (r["sessions"].as_u64(), r["skills"]["hit_rate"].is_null()),
        (Some(0), true)
    );
}

/// A runner, so the plan doesn't run everything and offer every skill, and
/// a guard rule, so the edit hook logs its call.
const ROUTES: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]

[guard.size]
files = ["src/**"]
file_lines = 100

[[skills.routes]]
skill = ".claude/skills/forms/SKILL.md"
paths = ["src/forms/**"]

[[skills.routes]]
skill = ".claude/skills/money/SKILL.md"
paths = ["src/money.ts"]
"#;

#[test]
fn a_report_over_events_the_brief_and_the_hooks_recorded() {
    let dir = repo("real", ROUTES);
    let (code, out) = fairlead(
        &dir,
        &[
            "brief",
            "--base",
            "main",
            "--session",
            "c1",
            "src/forms/form.ts",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let skill = json!({"session_id": "c1", "cwd": dir, "hook_event_name": "PostToolUse", "tool_name": "Skill", "tool_input": {"skill": "forms"}});
    hook(&dir, "nudge", skill);
    // A Codex session: its edit hook names its tool, and its nudge offers a skill it can't be seen loading.
    let file = dir.join("src/forms/form.ts");
    let patch = "*** Begin Patch\n*** Update File: src/forms/form.ts\n@@\n export const f = m;\n+export const g = m;\n*** End Patch\n";
    hook(
        &dir,
        "hook",
        json!({"session_id": "x1", "cwd": dir, "hook_event_name": "PreToolUse", "tool_name": "apply_patch", "tool_input": {"command": patch}}),
    );
    let (code, out) = fairlead(
        &dir,
        &["brief", "--base", "main", "--session", "x1", "src/money.ts"],
    );
    assert_eq!(code, 0, "{out}");
    hook(
        &dir,
        "nudge",
        json!({"session_id": "x2", "cwd": dir, "hook_event_name": "PostToolUse", "tool_name": "apply_patch", "tool_input": {"file_path": file}}),
    );

    let r = json_report(&dir, &[]);
    assert_eq!(
        rows(&r),
        [("forms".into(), 2, 1, 0, 0), ("money".into(), 2, 0, 0, 1)],
        "{r:#}"
    );
    assert_eq!(r["unmeasured"]["sessions"], json!(["x1", "x2"]));
    assert_eq!(
        (
            r["skills"]["sessions_offered"].as_u64(),
            r["skills"]["sessions_hit"].as_u64()
        ),
        (Some(1), Some(1))
    );
}
