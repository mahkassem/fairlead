//! Skill routing on a real repository: the brief's skills row by scope and
//! reach, capped; offers and uses in the event log; the nudge that names a
//! skill once per session; config check on routes; and `[skills]` never
//! moving a plan id.

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

const ROUTES: &str = r#"
[[skills.routes]]
skill = ".claude/skills/forms/SKILL.md"
paths = ["src/forms/**"]

[[skills.routes]]
skill = ".claude/skills/money/SKILL.md"
paths = ["src/money.ts"]

[[skills.routes]]
skill = ".claude/skills/style/SKILL.md"
always = true

[[skills.routes]]
skill = ".claude/skills/docs/SKILL.md"
paths = ["docs/**"]
"#;

fn skill(dir: &Path, name: &str, description: &str) {
    let d = dir.join(format!(".claude/skills/{name}"));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\nHow to.\n"),
    )
    .unwrap();
}

/// src/forms/form.ts imports src/money.ts.
fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-skills-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/forms")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("fairlead.toml"), config).unwrap();
    std::fs::write(dir.join("src/money.ts"), "export const m = 1;\n").unwrap();
    std::fs::write(
        dir.join("src/forms/form.ts"),
        "import { m } from '../money';\nexport const f = m;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("test/form.test.ts"),
        "import { f } from '../src/forms/form';\n",
    )
    .unwrap();
    for (n, d) in [
        ("forms", "How forms validate."),
        ("money", "Amounts are integers."),
        ("style", "House style."),
        ("docs", "Docs tone."),
    ] {
        skill(&dir, n, d);
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn skills_row(dir: &Path, path: &str) -> Vec<(String, String)> {
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
    brief["skills"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap().into(),
                i["why"].as_str().unwrap().into(),
            )
        })
        .collect()
}

fn events(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn hook(dir: &Path, call: Value) -> String {
    let mut child = command(dir, env!("CARGO_BIN_EXE_fairlead"), &["guard", "nudge"])
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
    assert!(out.status.success(), "the hook always exits 0");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_brief_routes_skills_named_then_used_then_always_with_their_description() {
    let dir = repo("route", &format!("{RUNNER}{ROUTES}"));
    let row = skills_row(&dir, "src/forms/form.ts");
    let names: Vec<&str> = row.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["forms", "money", "style"], "{row:?}");
    assert!(
        row[0]
            .1
            .starts_with("How forms validate. (named: src/forms/form.ts)"),
        "{row:?}"
    );
    assert!(
        row[1]
            .1
            .contains("used: src/forms/form.ts imports src/money.ts"),
        "{row:?}"
    );
    assert!(row[2].1.contains("(always)"), "{row:?}");

    let offers: Vec<Value> = events(&dir)
        .into_iter()
        .filter(|e| e["stage"] == "offer")
        .collect();
    assert_eq!(offers.len(), 1, "one offer event per brief");
    assert_eq!(offers[0]["session"], "s-src/forms/form.ts");
    assert_eq!(
        offers[0]["items"],
        json!(["skill:forms", "skill:money", "skill:style"])
    );

    // Importers are off by default, so the test that imports the form gets nothing extra.
    let names: Vec<String> = skills_row(&dir, "src/money.ts")
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["money", "style"]);
    let wider = repo(
        "importers",
        &format!("{RUNNER}[skills]\nimporters = 1\n{ROUTES}"),
    );
    let names: Vec<String> = skills_row(&wider, "src/money.ts")
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        names,
        ["money", "forms", "style"],
        "importers = 1 reaches the form"
    );
}

#[test]
fn the_skills_row_is_capped_and_a_missing_skill_is_named() {
    let mut routes = String::from(RUNNER);
    for i in 0..10 {
        routes.push_str(&format!(
            "\n[[skills.routes]]\nskill = \".claude/skills/s{i}/SKILL.md\"\npaths = [\"src/**\"]\n"
        ));
    }
    routes
        .push_str("\n[[skills.routes]]\nskill = \".claude/skills/gone/SKILL.md\"\nalways = true\n");
    let dir = repo("cap", &routes);
    for i in 0..10 {
        skill(&dir, &format!("s{i}"), "One of many.");
    }
    let (code, out) = fairlead(&dir, &["brief", "--base", "main", "src/money.ts"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("skills   10 skills"), "{out}");
    assert!(out.contains("… 2 more (fairlead brief --all)"), "{out}");
    assert!(
        out.contains("warning  [bad-skill] .claude/skills/gone/SKILL.md: can't read it"),
        "{out}"
    );
    assert!(out.lines().count() < 40, "{out}");
}

#[test]
fn an_edit_names_its_skill_once_per_session_and_a_skill_load_is_a_use() {
    let dir = repo("nudge", &format!("{RUNNER}{ROUTES}"));
    let edit = |session: &str| {
        let file = dir.join("src/forms/form.ts");
        hook(
            &dir,
            json!({"session_id": session, "cwd": dir, "hook_event_name": "PostToolUse", "tool_name": "Edit", "tool_input": {"file_path": file}}),
        )
    };
    let first: Value = serde_json::from_str(edit("s1").trim()).unwrap();
    let note = first["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(note.contains("no brief for this change yet"), "{note}");
    assert!(
        note.contains("the `forms` skill applies to src/forms/form.ts: How forms validate."),
        "{note}"
    );
    assert!(
        !note.contains("`money`"),
        "only the skill whose scope names the file: {note}"
    );
    assert_eq!(edit("s1").trim(), "", "said once per session");
    assert!(
        edit("s2").contains("`forms` skill"),
        "a new session hears it again"
    );

    // Claude Code's Skill tool, as its PostToolUse hook receives it.
    let skill_call = json!({"session_id": "s1", "transcript_path": "/tmp/t.jsonl", "cwd": dir, "hook_event_name": "PostToolUse", "tool_name": "Skill", "tool_input": {"skill": "forms", "args": ""}, "tool_response": {"success": true}});
    assert_eq!(
        hook(&dir, skill_call).trim(),
        "",
        "a use is recorded, never answered"
    );
    // Gemini CLI reading the skill file counts too.
    let read = json!({"session_id": "s3", "cwd": dir, "tool_name": "read_file", "tool_input": {"absolute_path": dir.join(".claude/skills/money/SKILL.md")}});
    hook(&dir, read);
    let uses: Vec<(String, Value)> = events(&dir)
        .into_iter()
        .filter(|e| e["stage"] == "use")
        .map(|e| {
            (
                e["session"].as_str().unwrap().to_string(),
                e["items"].clone(),
            )
        })
        .collect();
    assert_eq!(
        uses,
        [
            ("s1".into(), json!(["skill:forms"])),
            ("s3".into(), json!(["skill:money"]))
        ]
    );
}

#[test]
fn config_check_names_a_route_without_a_scope_a_wrong_file_and_a_duplicate() {
    let bad = format!(
        "{RUNNER}[skills]\ntargets = [\"claude\", \"vim\"]\n\n[[skills.routes]]\nskill = \"x/README.md\"\npaths = [\"src/**\"]\n\n[[skills.routes]]\nskill = \"a/SKILL.md\"\n\n[[skills.routes]]\nskill = \"a/SKILL.md\"\nalways = true\n"
    );
    let dir = repo("check", &bad);
    let (code, out) = fairlead(&dir, &["config", "check"]);
    assert_ne!(code, 0, "{out}");
    for want in [
        "`vim` isn't one of",
        "must be a SKILL.md path",
        "needs a scope",
        "routed twice",
    ] {
        assert!(out.contains(want), "{want}: {out}");
    }
}

#[test]
fn skill_routes_never_move_a_plan_id() {
    let digest = |config: &str, name: &str| {
        let dir = repo(name, config);
        std::fs::write(dir.join("src/money.ts"), "export const m = 2;\n").unwrap();
        let (code, out) = fairlead(&dir, &["plan", "--base", "main", "--json"]);
        assert_eq!(code, 0, "{out}");
        serde_json::from_str::<Value>(&out).unwrap()["config_digest"].clone()
    };
    assert_eq!(
        digest(RUNNER, "d1"),
        digest(&format!("{RUNNER}[skills]\ncap = 3\n{ROUTES}"), "d2")
    );
}
