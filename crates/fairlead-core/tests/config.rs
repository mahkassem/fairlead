use std::fs;
use std::path::{Path, PathBuf};

use fairlead_core::config::{
    load, validate, Config, Invoke, List, LoadOptions, TestClass, Unreached,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-config-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("nested/deeper")).unwrap();
    // A repository root, so discovery never climbs into the temp directory's parents.
    fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

fn repo_with(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = scratch(name);
    for (name, text) in files {
        fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn load_at(dir: &Path, opts: &LoadOptions) -> Config {
    load(dir, opts).unwrap().config
}

/// Each test names its own directory: tests run in parallel, and a shared
/// one gets removed under another test's feet.
fn copy_fixture(test: &str, name: &str, into: &str) -> PathBuf {
    let dir = scratch(test);
    fs::copy(Path::new(FIXTURES).join(name), dir.join(into)).unwrap();
    dir
}

#[test]
fn toml_and_yaml_fixtures_load_to_the_same_config() {
    let toml = load_at(
        &copy_fixture("same-toml", "full.toml", "fairlead.toml"),
        &LoadOptions::default(),
    );
    let yaml = load_at(
        &copy_fixture("same-yaml", "full.yaml", "fairlead.yaml"),
        &LoadOptions::default(),
    );
    assert_eq!(toml, yaml);
    assert_eq!(toml.tests.unreached, Unreached::All);
    assert_eq!(toml.tests.runners.items()[1].invoke, Invoke::PerModule);
    assert_eq!(toml.tests.classes.items()[0].class, TestClass::Demand);
    assert!(validate(&toml).is_empty(), "{:?}", validate(&toml));
}

#[test]
fn a_loaded_config_serializes_back_to_toml_and_yaml_unchanged() {
    let config = load_at(
        &copy_fixture("round-trip", "full.toml", "fairlead.toml"),
        &LoadOptions::default(),
    );
    let as_toml: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    let as_yaml: Config =
        serde_saphyr::from_str(&serde_saphyr::to_string(&config).unwrap()).unwrap();
    assert_eq!(as_toml, config);
    assert_eq!(as_yaml, config);
}

#[test]
fn project_lists_append_to_defaults_and_replace_replaces() {
    let config = load_at(
        &copy_fixture("lists", "full.toml", "fairlead.toml"),
        &LoadOptions::default(),
    );
    let run_all = config.plan.run_all.items();
    assert!(
        run_all.contains(&"pnpm-lock.yaml".to_string()),
        "defaults kept"
    );
    assert_eq!(
        run_all.last().map(String::as_str),
        Some("ci/**"),
        "project appended"
    );
    assert_eq!(
        config.graph.conditions,
        List::Items(vec!["source".into(), "import".into(), "default".into()])
    );
}

#[test]
fn later_layers_win_project_then_local_then_env_then_set() {
    let dir = repo_with(
        "precedence",
        &[
            (
                "fairlead.toml",
                "[tests]\nunreached = \"warn\"\n[replay]\nwindow_days = 10\nmin_failures = 1\n",
            ),
            (
                "fairlead.local.toml",
                "[tests]\nunreached = \"all\"\n[replay]\nwindow_days = 20\n",
            ),
        ],
    );
    let opts = LoadOptions {
        env: vec![("FAIRLEAD_REPLAY__WINDOW_DAYS".into(), "30".into())],
        sets: vec!["replay.min_failures=5".into()],
        ci: false,
    };
    let loaded = load(&dir.join("nested/deeper"), &opts).unwrap();
    assert_eq!(loaded.config.tests.unreached, Unreached::All);
    assert_eq!(loaded.config.replay.window_days, 30);
    assert_eq!(loaded.config.replay.min_failures, 5);
    assert_eq!(loaded.origins["tests.unreached"], "fairlead.local.toml");
    assert_eq!(
        loaded.origins["replay.window_days"],
        "env FAIRLEAD_REPLAY__WINDOW_DAYS"
    );
    assert_eq!(loaded.origins["replay.min_failures"], "--set");
    assert_eq!(loaded.origins["graph.tsconfig"], "default");
    assert_eq!(loaded.root, dir);
}

#[test]
fn the_local_file_is_ignored_in_ci() {
    let dir = repo_with(
        "ci",
        &[
            ("fairlead.toml", "[tests]\nunreached = \"warn\"\n"),
            ("fairlead.local.toml", "[tests]\nunreached = \"all\"\n"),
        ],
    );
    let config = load_at(
        &dir,
        &LoadOptions {
            ci: true,
            ..LoadOptions::default()
        },
    );
    assert_eq!(config.tests.unreached, Unreached::Warn);
}

#[test]
fn an_unknown_key_names_its_file_and_key() {
    let dir = repo_with(
        "unknown-key",
        &[("fairlead.toml", "[tests]\nunreachd = \"all\"\n")],
    );
    let err = load(&dir, &LoadOptions::default()).unwrap_err();
    assert_eq!(err.source, "fairlead.toml");
    assert_eq!(err.key.as_deref(), Some("tests.unreachd"));
    assert!(err.message.contains("unknown field"), "{err}");
}

#[test]
fn an_unknown_key_in_the_local_file_or_a_set_names_that_layer() {
    let dir = repo_with(
        "local-key",
        &[
            ("fairlead.toml", ""),
            ("fairlead.local.toml", "[plan]\nrunall = []\n"),
        ],
    );
    let err = load(&dir, &LoadOptions::default()).unwrap_err();
    assert_eq!(err.source, "fairlead.local.toml");
    let dir = repo_with("set-key", &[("fairlead.yaml", "{}\n")]);
    let err = load(
        &dir,
        &LoadOptions {
            sets: vec!["graph.tsconfg=x".into()],
            ..LoadOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        (err.source.as_str(), err.key.as_deref()),
        ("--set", Some("graph.tsconfg"))
    );
}

#[test]
fn a_wrong_value_names_the_key() {
    let dir = repo_with(
        "wrong-value",
        &[("fairlead.toml", "[tests]\nunreached = \"sometimes\"\n")],
    );
    let err = load(&dir, &LoadOptions::default()).unwrap_err();
    assert_eq!(err.key.as_deref(), Some("tests.unreached"));
}

#[test]
fn two_project_files_in_one_directory_is_an_error() {
    let dir = repo_with("two-files", &[("fairlead.toml", ""), ("fairlead.yaml", "")]);
    let err = load(&dir, &LoadOptions::default()).unwrap_err();
    assert!(err.message.contains("more than one config file"), "{err}");
}

#[test]
fn no_config_file_means_defaults() {
    let dir = scratch("none");
    let loaded = load(&dir, &LoadOptions::default()).unwrap();
    assert!(loaded.files.is_empty());
    assert_eq!(loaded.config, Config::default());
}

#[test]
fn semantic_problems_are_reported_without_failing_the_load() {
    let text = r#"
fairlead = "99.0"
[[modules.define]]
pattern = "services/*/src"
[[tests.runners]]
id = "unit"
match = ["**"]
command = ["x"]
[[tests.runners]]
id = "unit"
match = ["**"]
cwd = "{package}"
command = ["y"]
[[tests.owners]]
match = "e2e/**"
covers = ["apps/{name}/**"]
[[replay.failures]]
runner = "missing"
extractor = "regex"
"#;
    let dir = repo_with("semantic", &[("fairlead.toml", text)]);
    let loaded = load(&dir, &LoadOptions::default()).unwrap();
    let keys: Vec<&str> = loaded.problems.iter().map(|p| p.key.as_str()).collect();
    for expected in [
        "fairlead",
        "modules.define[0].pattern",
        "tests.runners",
        "tests.runners[1].cwd",
        "tests.owners[0].covers[0]",
        "replay.failures[0].pattern",
        "replay.failures[0].runner",
    ] {
        assert!(keys.contains(&expected), "missing {expected} in {keys:?}");
    }
}

#[test]
fn discovery_stops_at_the_repository_root() {
    let outer = scratch("outside-repo");
    fs::write(
        outer.join("fairlead.toml"),
        "[[checks]]\nid = \"x\"\ncommand = [\"y\"]\npaths = [\"**\"]\n",
    )
    .unwrap();
    let repo = outer.join("nested");
    fs::remove_dir_all(outer.join(".git")).unwrap();
    fs::create_dir_all(repo.join(".git")).unwrap();
    let loaded = load(&repo.join("deeper"), &LoadOptions::default()).unwrap();
    assert!(
        loaded.files.is_empty(),
        "read {:?} from outside the repository",
        loaded.files
    );
}

#[test]
fn a_value_that_should_be_a_list_says_so() {
    let dir = repo_with(
        "list-error",
        &[("fairlead.toml", "[plan]\nrun_all = \"x\"\n")],
    );
    let err = load(&dir, &LoadOptions::default()).unwrap_err();
    assert_eq!(err.key.as_deref(), Some("plan.run_all"));
    assert!(
        err.message.contains("a list, or { replace = [...] }"),
        "{err}"
    );
}

#[test]
fn a_set_can_replace_a_list() {
    let dir = repo_with(
        "set-replace",
        &[("fairlead.toml", "[plan]\nrun_all = [\"ci/**\"]\n")],
    );
    let opts = LoadOptions {
        sets: vec!["plan.run_all={ replace = [\"only/**\"] }".into()],
        ..LoadOptions::default()
    };
    let config = load(&dir, &opts).unwrap().config;
    assert_eq!(config.plan.run_all.items(), ["only/**".to_string()]);
}
