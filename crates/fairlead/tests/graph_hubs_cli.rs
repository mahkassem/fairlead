//! `fairlead graph hubs` on a repository whose widest file loads modules by
//! a name it computes, so the graph can't say which one it needs.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const FILES: &[(&str, &str)] = &[
    ("package.json", r#"{ "private": true }"#),
    (
        "src/loader.ts",
        "export const load = (name: string) => import(name);\n",
    ),
    (
        "src/storage.ts",
        "import { load } from './loader';\nexport const put = load;\n",
    ),
    (
        "src/mail.ts",
        "import { load } from './loader';\nexport const send = load;\n",
    ),
    ("src/util.ts", "export const u = 1;\n"),
    (
        "src/format.ts",
        "import { u } from './util';\nexport const f = u;\n",
    ),
    (
        "test/storage.test.ts",
        "import { put } from '../src/storage';\nput;\n",
    ),
    (
        "test/mail.test.ts",
        "import { send } from '../src/mail';\nsend;\n",
    ),
    (
        "test/format.test.ts",
        "import { f } from '../src/format';\nf;\n",
    ),
];

fn repo(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-hubs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    for (path, text) in FILES {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }
    std::fs::write(
        dir.join("fairlead.toml"),
        format!("[graph]\ncache = false\n{config}"),
    )
    .unwrap();
    dir
}

fn fairlead(dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn hubs_json(dir: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["graph", "hubs", "--json"];
    args.extend(extra);
    serde_json::from_str(&fairlead(dir, &args)).unwrap()
}

#[test]
fn a_file_with_a_dynamic_import_and_no_literal_path_is_the_top_hub_and_the_barrier_names_it() {
    let dir = repo("text", "");
    let text = fairlead(&dir, &["graph", "hubs"]);
    // Every other file feeds the loader through its whole-module edge, and
    // it reaches the two tests whose code imports it.
    let files = FILES.len() + 1;
    assert!(
        text.contains(&format!(
            " 1. src/loader.ts\n    reaches 2 tests, fed by {} files: {} pairs\n    wide: a dynamic import with no literal path, so it depends on its whole module\n",
            files - 1,
            2 * (files - 1)
        )),
        "{text}"
    );
    assert!(
        text.contains("wide: through src/loader.ts: a dynamic import"),
        "{text}"
    );
    assert!(
        text.contains("\n  graph.barrier = [\"src/loader.ts\"]\n"),
        "{text}"
    );
    assert!(text.contains("`fairlead test --explain TEST`"), "{text}");
}

#[test]
fn json_lists_the_same_hubs_and_top_limits_them() {
    let dir = repo("json", "");
    let all = hubs_json(&dir, &[]);
    let hubs = all["hubs"].as_array().unwrap();
    assert_eq!(hubs[0]["path"], "src/loader.ts");
    assert_eq!(hubs[0]["tests"], 2);
    assert_eq!(hubs[0]["why"]["kind"], "dynamic-import");
    assert_eq!(all["barrier"], "graph.barrier = [\"src/loader.ts\"]");
    let mail = hubs.iter().find(|h| h["path"] == "src/mail.ts").unwrap();
    assert_eq!(mail["why"]["through"], "src/loader.ts");
    assert_eq!(mail["score"], hubs[0]["score"]);
    let format = hubs.iter().find(|h| h["path"] == "src/format.ts").unwrap();
    assert_eq!(format["why"]["kind"], "plain");
    assert!(hubs.len() > 2, "{all}");

    let one = hubs_json(&dir, &["--top", "1"]);
    assert_eq!(one["hubs"].as_array().unwrap().len(), 1);
    assert_eq!(one["hubs"][0], hubs[0]);
}

#[test]
fn a_barrier_on_the_top_hub_takes_it_off_the_list_and_cuts_what_went_through_it() {
    let dir = repo("barrier", "barrier = [\"src/loader.ts\"]\n");
    let all = hubs_json(&dir, &[]);
    let hubs = all["hubs"].as_array().unwrap();
    assert!(hubs.iter().all(|h| h["path"] != "src/loader.ts"), "{all}");
    // What is left is one change feeding one test at a time.
    assert!(hubs.iter().all(|h| h["score"] == 1), "{all}");
}

#[test]
fn a_repository_with_no_test_reached_lists_no_hub_and_no_barrier() {
    let dir = repo("none", "");
    for test in ["storage", "mail", "format"] {
        std::fs::remove_file(dir.join(format!("test/{test}.test.ts"))).unwrap();
    }
    let text = fairlead(&dir, &["graph", "hubs"]);
    assert!(text.contains("no file links a change to a test"), "{text}");
    assert!(!text.contains("graph.barrier"), "{text}");
    assert!(hubs_json(&dir, &[])["barrier"].is_null());
}
