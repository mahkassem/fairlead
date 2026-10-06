//! `fairlead import rules` on a real repository: a dry run writes nothing,
//! `--write` writes the skills and appends routes `config check` accepts,
//! a second run changes nothing, and a different file in the way stops it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    // A clean environment, so a developer's FAIRLEAD_*, CI or git settings can't leak in.
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.env("HOME", dir).env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

fn fairlead(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

const CONFIG: &str = "[[skills.routes]]\nskill = \".claude/skills/style/SKILL.md\"\nalways = true";

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    write(&dir, "fairlead.toml", CONFIG);
    write(
        &dir,
        ".claude/skills/style/SKILL.md",
        "---\nname: style\n---\nTabs.\n",
    );
    write(
        &dir,
        ".claude/rules/api.md",
        "---\ndescription: How API handlers validate\npaths:\n  - \"src/api/**\"\n---\n# API\nValidate first.\n",
    );
    write(&dir, ".claude/rules/notes.md", "# Notes\nNo scope here.\n");
    write(
        &dir,
        ".cursor/rules/react.mdc",
        "---\ndescription: React components\nglobs: *.tsx, src/ui/**\nalwaysApply: false\n---\nUse hooks.\n",
    );
    write(
        &dir,
        ".cursor/rules/house.mdc",
        "---\nalwaysApply: true\n---\nShort functions. Small files.\n",
    );
    dir
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

#[test]
fn a_dry_run_prints_the_skills_and_routes_and_writes_nothing() {
    let dir = repo("dry");
    let (code, out) = fairlead(&dir, &["import", "rules", ".claude/rules"]);
    assert_eq!(code, 0, "{out}");
    for want in [
        "skill .claude/skills/api/SKILL.md from .claude/rules/api.md (src/api/**)",
        "How API handlers validate",
        "skill .claude/skills/notes/SKILL.md from .claude/rules/notes.md (always)",
        "# fairlead import rules .claude/rules\n[[skills.routes]]\nskill = \".claude/skills/api/SKILL.md\"\npaths = [\"src/api/**\"]",
        "nothing written",
        "rule files stay where they are",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    assert!(!dir.join(".claude/skills/api").exists());
    assert_eq!(read(&dir, "fairlead.toml"), CONFIG);
}

#[test]
fn write_adds_skills_and_routes_the_config_accepts_and_a_rerun_changes_nothing() {
    let dir = repo("write");
    let (code, out) = fairlead(&dir, &["import", "rules", ".claude/rules", "--write"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = fairlead(&dir, &["import", "rules", ".cursor/rules", "--write"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        read(&dir, ".claude/skills/api/SKILL.md"),
        "---\nname: api\ndescription: \"How API handlers validate\"\n---\n# API\nValidate first.\n"
    );
    assert!(
        read(&dir, ".claude/skills/house/SKILL.md").contains("description: \"Short functions.\"")
    );
    let config = read(&dir, "fairlead.toml");
    assert!(config.starts_with(CONFIG), "existing text kept:\n{config}");
    for want in [
        "# fairlead import rules .claude/rules\n",
        "# fairlead import rules .cursor/rules\n",
        "paths = [\"**/*.tsx\", \"src/ui/**\"]",
        "skill = \".claude/skills/house/SKILL.md\"\nalways = true",
    ] {
        assert!(config.contains(want), "missing {want:?} in:\n{config}");
    }
    assert!(dir.join(".claude/rules/api.md").exists(), "rule files stay");
    let (code, out) = fairlead(&dir, &["config", "check"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = fairlead(&dir, &["import", "rules", ".claude/rules", "--write"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("already routed: .claude/skills/api/SKILL.md"),
        "{out}"
    );
    assert!(out.contains("nothing to import"), "{out}");
    assert_eq!(
        read(&dir, "fairlead.toml"),
        config,
        "a rerun appends nothing"
    );
}

#[test]
fn a_different_skill_in_the_way_stops_the_write_before_anything_is_written() {
    let dir = repo("clash");
    write(&dir, ".claude/skills/react/SKILL.md", "someone else's\n");
    let (code, out) = fairlead(&dir, &["import", "rules", ".cursor/rules", "--write"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains(".claude/skills/react/SKILL.md exists with other content"),
        "{out}"
    );
    assert!(!dir.join(".claude/skills/house").exists());
    assert_eq!(read(&dir, "fairlead.toml"), CONFIG);
}
