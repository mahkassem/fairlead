use std::process::Command;

#[test]
fn version_flag_prints_the_package_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("fairlead {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn doctor_runs_and_names_the_platform() {
    let out = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .arg("doctor")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout)
        .contains(&format!("platform: {}-", std::env::consts::OS)));
}

fn fairlead_in(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    // A clean environment, so a developer's FAIRLEAD_* or CI variables can't leak in.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args(args).current_dir(dir).env_clear();
    for keep in ["PATH", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    cmd.output().unwrap()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

#[test]
fn config_check_fails_naming_the_file_and_key() {
    let dir = scratch("check-bad");
    std::fs::write(dir.join("fairlead.toml"), "[plan]\nrunall = []\n").unwrap();
    let out = fairlead_in(&dir, &["config", "check"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("fairlead.toml: plan.runall: unknown field"),
        "{err}"
    );
}

#[test]
fn config_check_passes_on_a_valid_yaml_file() {
    let dir = scratch("check-ok");
    std::fs::write(dir.join("fairlead.yaml"), "tests:\n  unreached: all\n").unwrap();
    let out = fairlead_in(&dir, &["config", "check"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("config ok: "));
}

#[test]
fn config_show_origin_names_the_layer_of_each_value() {
    let dir = scratch("show");
    std::fs::write(dir.join("fairlead.toml"), "[tests]\nunreached = \"warn\"\n").unwrap();
    let out = fairlead_in(
        &dir,
        &[
            "config",
            "show",
            "--origin",
            "--set",
            "graph.type_imports=false",
        ],
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("tests.unreached = \"warn\"  # fairlead.toml"),
        "{text}"
    );
    assert!(
        text.contains("graph.type_imports = false  # --set"),
        "{text}"
    );
    assert!(
        text.contains("graph.tsconfig = \"auto\"  # default"),
        "{text}"
    );
}

#[test]
fn config_schema_prints_a_json_schema() {
    let out = fairlead_in(&std::env::temp_dir(), &["config", "schema"]);
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["tests"].is_object());
}

#[test]
fn graph_why_prints_the_chain_and_importers_lists_the_edge() {
    let dir = scratch("graph");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.ts"), "import { b } from './b.js';\n").unwrap();
    std::fs::write(
        dir.join("src/b.ts"),
        "import { c } from './c';\nexport const b = 1;\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/c.ts"), "export const c = 1;\n").unwrap();
    let why = fairlead_in(&dir, &["graph", "why", "src/a.ts", "src/c.ts"]);
    let text = String::from_utf8_lossy(&why.stdout);
    assert!(
        why.status.success(),
        "{}",
        String::from_utf8_lossy(&why.stderr)
    );
    assert_eq!(
        text.lines().map(str::trim).collect::<Vec<_>>(),
        ["src/a.ts  (import)", "src/b.ts  (import)", "src/c.ts"]
    );
    let importers = fairlead_in(&dir.join("src"), &["graph", "importers", "c.ts"]);
    assert_eq!(
        String::from_utf8_lossy(&importers.stdout).trim(),
        "src/b.ts  (import)"
    );
    let stats = fairlead_in(&dir, &["graph", "stats", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&stats.stdout).unwrap();
    assert_eq!(value["sources"], 3);
    assert_eq!(value["edges"], 2);
}
