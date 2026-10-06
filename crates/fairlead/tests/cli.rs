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
    assert_eq!(value["cache"]["enabled"], true);
    let off = fairlead_in(&dir, &["graph", "stats", "--set", "graph.cache=false"]);
    assert!(String::from_utf8_lossy(&off.stdout).contains("parse cache: off"));
}

#[test]
fn replay_run_refuses_a_config_with_problems() {
    let dir = scratch("replay-bad");
    let config = dir.join("bench.toml");
    std::fs::write(
        &config,
        "[[replay.quarantine]]\npath = \"a.test.ts\"\njob = \"^win$\"\nreason = \"flaky\"\nuntil = \"2026/12/31\"\n",
    )
    .unwrap();
    let data = dir.join("data.jsonl");
    std::fs::write(&data, "").unwrap();
    let out = fairlead_in(
        &dir,
        &[
            "replay",
            "run",
            "--data",
            data.to_str().unwrap(),
            "--clone",
            dir.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("replay.quarantine"), "{err}");
}

#[test]
fn graph_why_says_where_a_barrier_stops_the_plan_and_stats_count_rule_edges() {
    let dir = scratch("graph-barrier");
    std::fs::create_dir_all(dir.join("src/http")).unwrap();
    std::fs::create_dir_all(dir.join("test")).unwrap();
    std::fs::write(dir.join("src/a.ts"), "import './http/server';\n").unwrap();
    std::fs::write(dir.join("src/http/server.ts"), "import '../c';\n").unwrap();
    std::fs::write(dir.join("src/c.ts"), "export const c = 1;\n").unwrap();
    std::fs::write(dir.join("test/a.test.ts"), "export {};\n").unwrap();
    std::fs::write(
        dir.join("fairlead.toml"),
        "[graph]\nbarrier = [\"src/http/**\"]\n\n[[graph.edges]]\nfrom = \"test/a.test.ts\"\nto = [\"src/a.ts\"]\n",
    )
    .unwrap();
    let why = fairlead_in(&dir, &["graph", "why", "src/a.ts", "src/c.ts"]);
    let text = String::from_utf8_lossy(&why.stdout);
    assert!(
        text.contains("its walk stops at src/http/server.ts"),
        "{text}"
    );
    let stats = fairlead_in(&dir, &["graph", "stats"]);
    let text = String::from_utf8_lossy(&stats.stdout);
    assert!(text.contains("rule edges: 1, barrier files: 1"), "{text}");
}

#[test]
fn graph_why_names_the_barrier_nearest_the_change_when_a_chain_crosses_two() {
    let dir = scratch("graph-two-barriers");
    std::fs::create_dir_all(dir.join("src/http")).unwrap();
    std::fs::create_dir_all(dir.join("src/db")).unwrap();
    std::fs::write(dir.join("src/a.ts"), "import './http/server';\n").unwrap();
    std::fs::write(dir.join("src/http/server.ts"), "import '../mid';\n").unwrap();
    std::fs::write(dir.join("src/mid.ts"), "import './db/pool';\n").unwrap();
    std::fs::write(dir.join("src/db/pool.ts"), "import '../c';\n").unwrap();
    std::fs::write(dir.join("src/c.ts"), "export const c = 1;\n").unwrap();
    std::fs::write(
        dir.join("fairlead.toml"),
        "[graph]\nbarrier = [\"src/http/**\", \"src/db/**\"]\n",
    )
    .unwrap();
    let why = fairlead_in(&dir, &["graph", "why", "src/a.ts", "src/c.ts"]);
    let text = String::from_utf8_lossy(&why.stdout);
    assert!(text.contains("its walk stops at src/db/pool.ts"), "{text}");
}

#[test]
fn env_flag_layers_that_environments_file_as_fairlead_env_does() {
    let dir = scratch("env-flag");
    std::fs::write(dir.join("fairlead.toml"), "").unwrap();
    std::fs::write(
        dir.join("fairlead.staging.toml"),
        "[tests]\nunreached = \"all\"\n",
    )
    .unwrap();
    let out = fairlead_in(&dir, &["config", "show", "--origin", "--env", "staging"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("tests.unreached = \"all\"  # fairlead.staging.toml"),
        "{text}"
    );
    let out = fairlead_in(&dir, &["--env", "prod", "config", "check"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no fairlead.prod.toml"));
    assert!(!fairlead_in(&dir, &["--env", "", "config", "check"])
        .status
        .success());
}

#[test]
fn replay_run_prints_progress_on_stderr_and_the_same_report_with_or_without_it() {
    let dir = std::env::temp_dir().join(format!("fairlead-cli-progress-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join("test")).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("src.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(repo.join("test/a.test.ts"), "import { a } from '../src';\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::write(repo.join("src.ts"), "export const a = 2;\n").unwrap();
    git(&["commit", "-q", "-am", "change"]);
    let head = git(&["rev-parse", "HEAD"]);
    let row = serde_json::json!({
        "repo": "example/repo", "run_id": 1, "attempt": 1, "event": "pull_request",
        "workflow": "ci", "pr": 1, "head_sha": head, "base_sha": base,
        "created_at": "2026-09-10T10:00:00Z", "conclusion": "failure",
        "jobs": [{ "name": "test", "conclusion": "failure", "log": [" FAIL  test/a.test.ts > a"] }]
    });
    let data = dir.join("data.jsonl");
    std::fs::write(&data, format!("{row}\n")).unwrap();
    let config = dir.join("bench.toml");
    std::fs::write(
        &config,
        "[graph]\ncache = false\n\n[[tests.runners]]\nid = \"vitest\"\nmatch = [\"**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n\n[[replay.failures]]\nrunner = \"vitest\"\nextractor = \"vitest\"\njob = \"^test$\"\n",
    )
    .unwrap();
    let run = |quiet: bool, json: &std::path::Path| {
        let mut args = vec![
            "replay",
            "run",
            "--data",
            data.to_str().unwrap(),
            "--clone",
            repo.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json-out",
            json.to_str().unwrap(),
        ];
        if quiet {
            args.push("--quiet");
        }
        let out = fairlead_in(&dir, &args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        (
            out.stdout,
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (loud, err) = run(false, &dir.join("loud.json"));
    let (quiet, quiet_err) = run(true, &dir.join("quiet.json"));
    assert!(
        err.contains("1 of 1 runs planned · 1 failures judged · 0 misses"),
        "{err}"
    );
    assert!(!quiet_err.contains("runs planned"), "{quiet_err}");
    // Planning time is measured, so it differs between any two runs.
    let untimed = |s: &str| {
        s.lines()
            .filter(|l| !l.contains("plan time:") && !l.contains("_seconds"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    assert_eq!(
        untimed(&text(&loud)),
        untimed(&text(&quiet)),
        "stdout doesn't change"
    );
    let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
    assert_eq!(
        untimed(&read("loud.json")),
        untimed(&read("quiet.json")),
        "the JSON report doesn't change"
    );
}

#[cfg(unix)]
#[test]
fn a_reader_that_stops_early_ends_the_run_quietly_with_the_sigpipe_status() {
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .args(["config", "schema"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert_eq!(out.status.code(), Some(141), "{err}");
}
