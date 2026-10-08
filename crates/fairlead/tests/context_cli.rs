//! `fairlead context` and `fairlead resume` on a real repository: the
//! context's bodies, paths, README and history under its line cap; resume
//! after a brief and an edit, and with no brief; the SessionStart hook's
//! answer; and the hook through install, status, uninstall and migrate.

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

const RUNNER: &str = r#"
[[tests.runners]]
id = "unit"
match = ["test/**"]
command = ["sh", "-c", "exit 0", "sh", "{files}"]
"#;

fn write(dir: &Path, path: &str, text: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

fn lesson(dir: &Path, id: &str, scope: &str, body: &str) {
    write(
        dir,
        &format!(".fairlead/lessons/{id}.md"),
        &format!(
            "---\nid: {id}\ntitle: \"Lesson {id}\"\npaths: [\"{scope}\"]\nadded: 2026-09-12\nevidence: [\"https://example.com/pr/1\"]\nsource: mistake\n---\n{body}\n"
        ),
    );
}

fn skill(dir: &Path, name: &str) {
    write(
        dir,
        &format!(".claude/skills/{name}/SKILL.md"),
        &format!("---\nname: {name}\ndescription: How {name} works.\n---\nHow to.\n"),
    );
}

fn route(name: &str, scope: &str) -> String {
    format!(
        "\n[[skills.routes]]\nskill = \".claude/skills/{name}/SKILL.md\"\npaths = [\"{scope}\"]\n"
    )
}

/// src/forms/form.ts imports src/money.ts; a test imports the form.
fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-context-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", config);
    write(&dir, "src/money.ts", "export const m = 1;\n");
    write(
        &dir,
        "src/forms/form.ts",
        "import { m } from '../money';\nexport const f = m;\n",
    );
    write(
        &dir,
        "test/form.test.ts",
        "import { f } from '../src/forms/form';\n",
    );
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

#[test]
fn context_stays_under_120_lines_and_says_how_many_more() {
    let mut config = RUNNER.to_string();
    for i in 0..12 {
        config.push_str(&route(&format!("s{i}"), "src/**"));
    }
    let dir = repo("cap", &config);
    for i in 0..12 {
        skill(&dir, &format!("s{i}"));
        let body: Vec<String> = (0..10)
            .map(|n| format!("Line {n} of lesson {i}."))
            .collect();
        lesson(&dir, &format!("l{i:02}"), "src/**", &body.join("\n"));
    }
    let (code, out) = fairlead(&dir, &["context", "src/forms/form.ts"]);
    assert_eq!(code, 0, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.len() <= 120, "{} lines:\n{out}", lines.len());
    let last = lines.last().unwrap();
    assert!(
        last.starts_with("… ") && last.ends_with(" more (fairlead context --all)"),
        "{last}"
    );
    assert!(out.contains("lesson   l00"), "{out}");

    let (code, all) = fairlead(&dir, &["context", "--all", "src/forms/form.ts"]);
    assert_eq!(code, 0, "{all}");
    assert!(all.lines().count() > 120, "{all}");
    assert!(!all.contains("(fairlead context --all)"), "{all}");
    assert!(all.contains("Line 9 of lesson 11."), "{all}");
    assert!(all.contains("load it before editing: .claude/skills/s11/SKILL.md"));
}

#[test]
fn context_prints_lesson_bodies_skill_paths_the_readme_and_history() {
    let config = format!("{RUNNER}{}", route("forms", "src/forms/**"));
    let dir = repo("bodies", &config);
    skill(&dir, "forms");
    lesson(
        &dir,
        "forms-validate-on-blur",
        "src/forms/**",
        "Validate on blur, never on each keystroke.",
    );
    write(
        &dir,
        "src/forms/README.md",
        "# Forms\n\nEvery form posts through one helper.\n\n## Later\nNot shown.\n",
    );
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "explain the forms"]);
    write(
        &dir,
        "src/forms/form.ts",
        "import { m } from '../money';\nexport const f = m + 1;\n",
    );
    git(&dir, &["commit", "-q", "-am", "make the form add one"]);

    let (code, out) = fairlead(&dir, &["context", "--session", "s1", "src/forms/form.ts"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("brief b-"), "the brief comes first: {out}");
    assert!(
        out.contains("lesson   forms-validate-on-blur  Lesson forms-validate-on-blur (named: src/forms/form.ts)"),
        "{out}"
    );
    assert!(
        out.contains("  Validate on blur, never on each keystroke."),
        "{out}"
    );
    assert!(out.contains("skill    forms  How forms works."), "{out}");
    assert!(
        out.contains("  load it before editing: .claude/skills/forms/SKILL.md"),
        "{out}"
    );
    assert!(out.contains("docs     src/forms/README.md  Forms"), "{out}");
    assert!(out.contains("  Every form posts through one helper."));
    assert!(!out.contains("Not shown."), "only the first section: {out}");
    let history: Vec<&str> = out
        .lines()
        .skip_while(|l| *l != "history  src/forms/form.ts")
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .collect();
    assert_eq!(history.len(), 2, "{out}");
    assert!(
        history[0].ends_with(" make the form add one"),
        "{history:?}"
    );
    assert!(history[1].ends_with(" init"), "{history:?}");

    let (code, out) = fairlead(&dir, &["context", "--json", "src/forms/form.ts"]);
    assert_eq!(code, 0, "{out}");
    let ctx: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        ctx["lessons"][0]["path"],
        ".fairlead/lessons/forms-validate-on-blur.md"
    );
    assert_eq!(ctx["skills"][0]["path"], ".claude/skills/forms/SKILL.md");
    assert_eq!(ctx["docs"][0]["heading"], "Forms");
    assert_eq!(ctx["history"][0]["commits"].as_array().unwrap().len(), 2);
}

/// A repository on a branch with a brief for src/money.ts, then that file
/// edited, a file outside the brief and a lesson added.
fn resumable(name: &str) -> PathBuf {
    let dir = repo(name, RUNNER);
    git(&dir, &["checkout", "-q", "-b", "feat"]);
    let (code, out) = fairlead(&dir, &["brief", "--session", "s1", "src/money.ts"]);
    assert_eq!(code, 0, "{out}");
    write(&dir, "src/money.ts", "export const m = 2;\n");
    write(&dir, "src/other.ts", "export const o = 1;\n");
    lesson(
        &dir,
        "money-is-integers",
        "src/money.ts",
        "Amounts are integers.",
    );
    dir
}

fn brief_id(dir: &Path) -> String {
    let file = std::fs::read_dir(dir.join(".git/fairlead/briefs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    file.file_stem().unwrap().to_string_lossy().into_owned()
}

#[test]
fn resume_after_a_brief_and_an_edit_names_the_brief_the_counts_and_next() {
    let dir = resumable("resume");
    let id = brief_id(&dir);
    // A new session has no brief of its own, so it gets the newest on the branch.
    for args in [&["resume"][..], &["resume", "--session", "s2"]] {
        let (code, out) = fairlead(&dir, args);
        assert_eq!(code, 0, "{out}");
        assert!(out.lines().count() <= 20, "{out}");
        assert!(
            out.starts_with(&format!("resume   brief {id}  base ")),
            "{out}"
        );
        assert!(out.contains("(the newest on this branch)"), "{out}");
        assert!(out.contains("  paths  src/money.ts\n"), "{out}");
        assert!(
            out.contains("changed  3 files since its base: 1 in the brief, 2 outside"),
            "{out}"
        );
        assert!(out.contains("gate     not run for this tree"), "{out}");
        assert!(
            out.contains("lessons  1 added on this branch")
                && out.contains("money-is-integers")
                && out.contains("Lesson money-is-integers"),
            "{out}"
        );
        assert!(out.contains("next     done: `fairlead done`"), "{out}");
    }
    let (_, out) = fairlead(&dir, &["resume", "--session", "s1", "--json"]);
    let r: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(r["found_by"], "session");
    assert_eq!(r["branch"], "feat");
    assert_eq!(r["in_brief"], json!(["src/money.ts"]));
    assert_eq!(r["gate"]["state"], "not run");

    // On another branch the brief isn't this branch's.
    git(&dir, &["checkout", "-q", "-b", "other"]);
    let (_, out) = fairlead(&dir, &["resume"]);
    assert!(
        out.starts_with("resume: no brief on this branch yet"),
        "{out}"
    );
}

#[test]
fn resume_with_no_brief_prints_one_line() {
    let dir = repo("nobrief", RUNNER);
    let (code, out) = fairlead(&dir, &["resume"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.contains("`fairlead brief <paths>`"), "{out}");
}

fn session_start(dir: &Path, stdin: &str) -> String {
    let mut child = command(dir, env!("CARGO_BIN_EXE_fairlead"), &["resume", "--hook"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "the hook always exits 0");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_session_start_hook_answers_with_resume_and_is_silent_without_a_brief() {
    let dir = resumable("hook");
    let id = brief_id(&dir);
    let call = json!({
        "session_id": "s-new",
        "transcript_path": "/tmp/t.jsonl",
        "cwd": dir,
        "hook_event_name": "SessionStart",
        "source": "startup"
    });
    let out = session_start(&dir, &call.to_string());
    let answer: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(
        answer["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    let context = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        context.contains(&format!("resume   brief {id}")),
        "{context}"
    );
    assert!(context.contains("next     "), "{context}");
    assert_eq!(answer.as_object().unwrap().len(), 1, "{answer}");

    let empty = repo("hookempty", RUNNER);
    let call = json!({"session_id": "s", "cwd": empty, "source": "startup"});
    assert_eq!(session_start(&empty, &call.to_string()), "");
    assert_eq!(session_start(&empty, "not json"), "");
}

fn settings(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(".claude/settings.json")).unwrap())
        .unwrap()
}

#[test]
fn hooks_install_status_and_uninstall_include_and_remove_session_start() {
    let dir = repo("install", RUNNER);
    let (code, out) = fairlead(&dir, &["hooks", "install"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("a new session starts with `fairlead resume`"),
        "{out}"
    );
    let after = settings(&dir);
    let group = &after["hooks"]["SessionStart"][0];
    assert!(group.get("matcher").is_none(), "every source: {group}");
    let command = group["hooks"][0]["command"].as_str().unwrap();
    assert!(command.contains("fairlead resume --hook"), "{command}");
    let (_, out) = fairlead(&dir, &["hooks", "status"]);
    assert!(out.contains("SessionStart"), "{out}");
    let (_, out) = fairlead(&dir, &["doctor"]);
    assert!(out.contains("PostToolUse, Stop, SessionStart"), "{out}");

    // Hooks installed before it get it from migrate.
    let mut older = after.clone();
    older["hooks"]
        .as_object_mut()
        .unwrap()
        .remove("SessionStart");
    older["hooks"]["PostToolUse"][0]["matcher"] = json!("Edit|Write|MultiEdit");
    write(&dir, ".claude/settings.json", &older.to_string());
    let (_, out) = fairlead(&dir, &["migrate"]);
    assert!(out.contains("adds the session resume"), "{out}");
    assert!(
        out.contains("the brief nudge matches `Edit|Write|MultiEdit|Skill` instead of `Edit|Write|MultiEdit`"),
        "{out}"
    );
    let (code, out) = fairlead(&dir, &["migrate", "--write"]);
    assert_eq!(code, 0, "{out}");
    let upgraded = settings(&dir);
    assert!(upgraded["hooks"]["SessionStart"].is_array());
    assert_eq!(
        upgraded["hooks"]["PostToolUse"][0]["matcher"],
        "Edit|Write|MultiEdit|Skill"
    );

    let (code, out) = fairlead(&dir, &["hooks", "uninstall"]);
    assert_eq!(code, 0, "{out}");
    let left = std::fs::read_to_string(dir.join(".claude/settings.json")).unwrap_or_default();
    assert!(!left.contains("SessionStart"), "{left}");
    assert!(!left.contains("fairlead"), "{left}");

    // Codex and Gemini CLI take the same hook and read the same answer;
    // Gemini CLI counts its timeout in milliseconds.
    for flag in ["--codex", "--gemini"] {
        let (code, out) = fairlead(&dir, &["hooks", "install", flag]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("fairlead resume"), "{out}");
    }
    for (file, timeout) in [(".codex/hooks.json", 30), (".gemini/settings.json", 30_000)] {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        let hooks: serde_json::Value = serde_json::from_str(&text).unwrap();
        let hook = &hooks["hooks"]["SessionStart"][0]["hooks"][0];
        assert!(
            hook["command"].as_str().unwrap().ends_with("resume --hook"),
            "{file}: {text}"
        );
        assert_eq!(hook["timeout"], timeout, "{file}: {text}");
    }
}

#[test]
fn brief_resume_false_installs_no_session_start_hook() {
    let dir = repo("off", &format!("{RUNNER}[brief]\nresume = false\n"));
    let (code, out) = fairlead(&dir, &["hooks", "install"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("fairlead resume"), "{out}");
    let after = settings(&dir);
    assert!(after["hooks"].get("SessionStart").is_none(), "{after}");
    assert!(after["hooks"]["Stop"].is_array(), "the rest is installed");
}
