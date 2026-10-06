//! `fairlead skills eval` on a synthetic history: which commits count, and
//! the recall each method gets, including a skill only imports reach and
//! one only importers reach.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    // A clean environment, so a developer's FAIRLEAD_*, CI or git settings can't leak in.
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

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

const ROUTES: &str = r#"
[[skills.routes]]
skill = ".claude/skills/money/SKILL.md"
paths = ["src/money.ts"]

[[skills.routes]]
skill = ".claude/skills/forms/SKILL.md"
paths = ["src/forms/**"]

[[skills.routes]]
skill = ".claude/skills/tests/SKILL.md"
paths = ["test/**"]

[[skills.routes]]
skill = ".claude/skills/style/SKILL.md"
always = true

[[skills.routes]]
skill = ".claude/skills/docs/SKILL.md"
paths = ["docs/**"]
"#;

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn touch(dir: &Path, rel: &str) {
    let mut text = std::fs::read_to_string(dir.join(rel)).unwrap_or_default();
    text.push_str("// more\n");
    write(dir, rel, &text);
}

fn commit(dir: &Path, edits: &[&str], message: &str) {
    for e in edits {
        touch(dir, e);
    }
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn skill(name: &str) -> String {
    format!(".claude/skills/{name}/SKILL.md")
}

/// src/forms/form.ts imports src/money.ts; test/handler.test.ts imports
/// src/api/handler.ts. Three commits count: one that needed money (only
/// imports reach it), one that needed tests (only importers do), and one
/// that needed forms (named).
fn repo() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-eval-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", ROUTES);
    write(&dir, "src/money.ts", "export const m = 1;\n");
    write(
        &dir,
        "src/forms/form.ts",
        "import { m } from '../money';\nexport const f = m;\n",
    );
    write(&dir, "src/api/handler.ts", "export const h = 1;\n");
    write(
        &dir,
        "test/handler.test.ts",
        "import { h } from '../src/api/handler';\n",
    );
    for name in ["money", "forms", "tests", "style"] {
        write(
            &dir,
            &skill(name),
            &format!("---\nname: {name}\ndescription: {name}\n---\nHow to.\n"),
        );
    }
    commit(&dir, &[], "init");
    commit(
        &dir,
        &["src/forms/form.ts", &skill("money")],
        "money via imports",
    );
    commit(
        &dir,
        &["src/api/handler.ts", &skill("tests")],
        "tests via importers",
    );
    commit(&dir, &["src/forms/form.ts", &skill("forms")], "forms named");
    commit(&dir, &[&skill("style")], "a skill alone");
    commit(&dir, &["src/money.ts"], "code alone");
    write(&dir, &skill("docs"), "---\nname: docs\n---\nNew.\n");
    commit(&dir, &["src/money.ts"], "a skill is born");
    dir
}

#[test]
fn eval_scores_each_method_on_the_commits_that_changed_code_and_a_skill() {
    let dir = repo();
    let (code, out) = fairlead(&dir, &["skills", "eval", "--json"]);
    assert_eq!(code, 0, "{out}");
    let report: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["walked"], 7);
    assert_eq!(
        report["scored"], 3,
        "a skill alone, code alone and a new skill don't count"
    );
    let method = |name: &str| {
        report["methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["method"] == name)
            .unwrap()
            .clone()
    };
    // (hit, needed, covered, offered) per method.
    for (name, want) in [
        ("paths", (1, 3, 1, 5)),
        ("imports", (2, 3, 2, 7)),
        ("importers", (2, 3, 2, 6)),
        ("both", (3, 3, 3, 8)),
    ] {
        let m = method(name);
        let got = (
            m["hit"].as_u64().unwrap(),
            m["needed"].as_u64().unwrap(),
            m["covered"].as_u64().unwrap(),
            m["offered"].as_u64().unwrap(),
        );
        assert_eq!(got, want, "{name}: {m}");
    }
    assert_eq!(method("paths")["recall"], 0.3333);
    assert_eq!(method("imports")["recall"], 0.6667);
    assert_eq!(method("both")["precision"], 0.375);
    assert_eq!(report["default"], "imports");
    let caps: Vec<(u64, f64)> = report["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["cap"].as_u64().unwrap(), c["recall"].as_f64().unwrap()))
        .collect();
    assert_eq!(caps, [(5, 0.6667), (6, 0.6667), (8, 0.6667), (10, 0.6667)]);

    let (code, text) = fairlead(&dir, &["skills", "eval"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("3 of 7 first-parent commits"), "{text}");
    assert!(text.contains("built once"), "{text}");
    assert!(text.contains("imports*"), "{text}");

    let (code, out) = fairlead(&dir, &["skills", "eval", "--limit", "2", "--json"]);
    assert_eq!(code, 0, "{out}");
    let report: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (report["walked"].as_u64(), report["scored"].as_u64()),
        (Some(2), Some(0))
    );
}
