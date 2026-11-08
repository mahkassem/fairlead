//! Untracked build output in a plan: a directory tagged as a cache stays
//! out of the change set and the tree hash, and a file that isn't source
//! reaching a dynamic import's whole module says how to keep it out.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(dir: &Path, program: &str, args: &[&str]) -> String {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    let out = cmd
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

const TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n# made by a build tool\n";

/// A root-level test whose import has no literal path, so it depends on
/// every source file outside any module.
fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-cachedir-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    run(&dir, "git", &["init", "-q", "-b", "main"]);
    write(
        &dir,
        "fairlead.toml",
        "[[tests.runners]]\nid = \"unit\"\nmatch = [\"test/**\"]\ncommand = [\"sh\", \"-c\", \"exit 0\", \"sh\", \"{files}\"]\n",
    );
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(
        &dir,
        "test/load.test.ts",
        "const name = process.argv[2];\nawait import(`../src/${name}`);\n",
    );
    run(&dir, "git", &["add", "-A"]);
    run(&dir, "git", &["commit", "-q", "-m", "base"]);
    write(&dir, ".build/CACHEDIR.TAG", TAG);
    write(&dir, ".build/.buildSystem_debug", "one\n");
    write(&dir, ".build/out/page.html", "<p>generated</p>\n");
    dir
}

fn plan(dir: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["plan", "--base", "main", "--json"];
    args.extend_from_slice(extra);
    serde_json::from_str(&run(dir, env!("CARGO_BIN_EXE_fairlead"), &args)).unwrap()
}

fn changed(plan: &serde_json::Value) -> Vec<String> {
    plan["changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap().to_string())
        .collect()
}

fn tests(plan: &serde_json::Value) -> Vec<String> {
    plan["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn an_untracked_cache_directory_is_no_change_and_leaves_the_tree_hash_alone() {
    let dir = repo("hash");
    let before = plan(&dir, &[]);
    assert!(changed(&before).is_empty(), "{before}");
    assert!(tests(&before).is_empty(), "{before}");
    let head_tree = run(&dir, "git", &["rev-parse", "HEAD^{tree}"]);
    assert_eq!(
        before["tree_hash"].as_str().unwrap(),
        head_tree.trim(),
        "a tree with only a cache untracked is HEAD's tree"
    );
    write(&dir, ".build/.buildSystem_debug", "two\n");
    write(&dir, "src/a.ts", "export const a = 2;\n");
    let edited = plan(&dir, &[]);
    assert_eq!(changed(&edited), ["src/a.ts"]);
    write(&dir, ".build/.buildSystem_debug", "three\n");
    write(&dir, ".build/more.html", "<p>more</p>\n");
    let again = plan(&dir, &[]);
    assert_eq!(
        again["tree_hash"], edited["tree_hash"],
        "another tool writing into the cache keeps a pass current"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_isnt_source_reaching_a_dynamic_import_says_how_to_keep_it_out() {
    let dir = repo("module");
    write(&dir, "notes/.scratch", "a note\n");
    let noted = plan(&dir, &[]);
    assert_eq!(changed(&noted), ["notes/.scratch"]);
    assert_eq!(tests(&noted), ["test/load.test.ts"], "{noted}");
    let why = run(
        &dir,
        env!("CARGO_BIN_EXE_fairlead"),
        &["test", "--base", "main", "--explain", "test/load.test.ts"],
    );
    assert!(
        why.contains("so it depends on its whole module, which takes in notes/.scratch too though it isn't source; if it's build output, list it in .gitignore or tag its folder with a CACHEDIR.TAG"),
        "{why}"
    );
    write(&dir, "src/new.ts", "export const n = 1;\n");
    let why = run(
        &dir,
        env!("CARGO_BIN_EXE_fairlead"),
        &[
            "test",
            "--explain",
            "test/load.test.ts",
            "--files",
            "src/new.ts",
        ],
    );
    assert!(
        why.ends_with("so it depends on its whole module)\n"),
        "a source file reads as before: {why}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
