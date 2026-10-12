//! A workspace folder of three repositories: `plan` from the folder plans
//! each against its own base, and a command inside one sees the workspace
//! file as a layer under its own.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fairlead_in(dir: &Path, args: &[&str], ci: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT", "HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    if ci {
        cmd.env("CI", "true");
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
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

const WORKSPACE: &str = "[workspace]\n\n[[workspace.repos]]\npath = \"api\"\nbase = \"main\"\n\n[[workspace.repos]]\npath = \"web\"\nbase = \"main\"\n\n[[workspace.repos]]\npath = \"libs/shared\"\nbase = \"main\"\n\n[[tests.runners]]\nid = \"vitest\"\nmatch = [\"**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n";

/// A folder with the workspace file and three committed repositories, none
/// with a config of its own; `api` and `web` have a change in the tree.
fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-ws-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "fairlead.toml", WORKSPACE);
    for repo in ["api", "web", "libs/shared"] {
        let r = dir.join(repo);
        std::fs::create_dir_all(&r).unwrap();
        git(&r, &["init", "-q", "-b", "main"]);
        write(&r, "src/a.ts", "export const a = 1;\n");
        write(&r, "test/a.test.ts", "import { a } from '../src/a';\n");
        git(&r, &["add", "-A"]);
        git(&r, &["commit", "-q", "-m", "init"]);
    }
    write(&dir, "api/src/a.ts", "export const a = 2;\n");
    write(&dir, "web/src/a.ts", "export const a = 3;\n");
    dir
}

fn json(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn a_plan_from_the_folder_covers_each_changed_repository_against_its_own_base() {
    let dir = workspace("all");
    let plan = json(&fairlead_in(&dir, &["plan", "--json"], false));
    let names: Vec<&str> = plan["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["api", "web"]);
    assert_eq!(plan["unchanged"], serde_json::json!(["shared"]));
    let api = &plan["repos"][0]["plan"];
    assert_eq!(api["changed"][0]["path"], "src/a.ts");
    assert_eq!(api["tests"][0]["path"], "test/a.test.ts");
    let text = fairlead_in(&dir, &["plan"], false);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("api (api)") && text.contains("unchanged: shared"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repo_narrows_the_plan_and_files_go_to_the_repository_holding_them() {
    let dir = workspace("narrow");
    let plan = json(&fairlead_in(
        &dir,
        &["plan", "--json", "--repo", "web"],
        false,
    ));
    assert_eq!(plan["repos"].as_array().unwrap().len(), 1);
    assert_eq!(plan["repos"][0]["name"], "web");
    let plan = json(&fairlead_in(
        &dir,
        &["plan", "--json", "--files", "libs/shared/src/a.ts"],
        false,
    ));
    assert_eq!(plan["repos"][0]["name"], "shared");
    assert_eq!(plan["repos"][0]["path"], "libs/shared");
    assert_eq!(plan["unchanged"], serde_json::json!(["api", "web"]));
    let out = fairlead_in(&dir, &["plan", "--repo", "nope"], false);
    assert!(String::from_utf8_lossy(&out.stderr).contains("no repository `nope`"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inside_a_repository_the_workspace_file_is_a_layer_under_its_own() {
    let dir = workspace("inside");
    let plan = json(&fairlead_in(
        &dir.join("api/src"),
        &["plan", "--json", "--base", "main"],
        false,
    ));
    assert!(
        plan.get("repos").is_none(),
        "one repository plans as before"
    );
    assert_eq!(
        plan["tests"][0]["runner"], "vitest",
        "the runner came from the workspace file"
    );
    let plan = json(&fairlead_in(
        &dir.join("api"),
        &["plan", "--json", "--base", "main"],
        true,
    ));
    assert!(
        plan["tests"][0].get("runner").is_none(),
        "CI reads no workspace file"
    );
    write(&dir, "api/fairlead.toml", "[tests.runners]\nreplace = [{ id = \"jest\", match = [\"**\"], command = [\"jest\", \"{files}\"] }]\n");
    let plan = json(&fairlead_in(
        &dir.join("api"),
        &["plan", "--json", "--base", "main"],
        false,
    ));
    assert_eq!(
        plan["tests"][0]["runner"], "jest",
        "the repository's file comes after the workspace's"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn other_commands_at_the_folder_and_a_repository_declaring_a_workspace_are_refused() {
    let dir = workspace("refused");
    let out = fairlead_in(&dir, &["config", "check"], false);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("workspace of 3 repositories (api, web, shared)"),
        "{err}"
    );
    write(&dir, "web/fairlead.toml", "[workspace]\n");
    let out = fairlead_in(&dir.join("web"), &["config", "check"], false);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && err.contains("only a workspace file"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn listing_no_repositories_discovers_those_one_level_down_and_explain_finds_the_repository() {
    let dir = workspace("discover");
    write(&dir, "fairlead.toml", "[workspace]\n\n[[tests.runners]]\nid = \"vitest\"\nmatch = [\"**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n");
    let plan = json(&fairlead_in(
        &dir,
        &["plan", "--json", "--base", "main"],
        false,
    ));
    let names: Vec<&str> = plan["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["api", "web"], "libs/shared is two levels down");
    assert!(plan["unchanged"].as_array().unwrap().is_empty());
    let out = fairlead_in(
        &dir,
        &["test", "--base", "main", "--explain", "web/test/a.test.ts"],
        false,
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("src/a.ts"),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
