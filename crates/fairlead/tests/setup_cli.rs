//! `fairlead setup` on temporary git repositories: what it applies, the
//! checklist it prints, the exit code, and that `--check` and `--dry-run`
//! leave the disk alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A clean environment, so a developer's FAIRLEAD_*, CI or git settings
/// can't leak in. The PATH starts with the repository's stub folder, where
/// a test puts its own `lefthook`, then the folder of the binary under test.
fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    if let Some(value) = std::env::var_os("SYSTEMROOT") {
        cmd.env("SYSTEMROOT", value);
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_fairlead")).parent().unwrap();
    let rest = std::env::var_os("PATH").unwrap_or_default();
    let path = [stubs(dir), binary.to_path_buf()]
        .into_iter()
        .chain(std::env::split_paths(&rest));
    cmd.env("PATH", std::env::join_paths(path).unwrap())
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    cmd
}

/// Beside the repository, so nothing in it is a file of the repository's.
fn stubs(dir: &Path) -> PathBuf {
    dir.with_extension("stubs")
}

/// The exit code, stdout and stderr of `fairlead setup` with `args`.
fn setup(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let mut all = vec!["setup"];
    all.extend_from_slice(args);
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), &all)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A git repository holding `files`, committed once.
fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-setup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(stubs(&dir));
    std::fs::create_dir_all(stubs(&dir)).unwrap();
    for (path, content) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["add", "-A"],
        &["commit", "-q", "-m", "base"],
    ] {
        let out = command(&dir, "git", args).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    dir
}

/// A `lefthook` whose `install` puts its runner in the git hooks, or fails.
/// A shell writes it: a file this process had open for writing can still
/// be open in another test's child when it is run, which fails the run.
#[cfg(unix)]
fn lefthook(dir: &Path, works: bool) {
    let script = match works {
        true => "#!/bin/sh\nmkdir -p .git/hooks\necho 'lefthook run pre-commit' > .git/hooks/pre-commit\n",
        false => "#!/bin/sh\necho 'lefthook: no permission' >&2\nexit 1\n",
    };
    let path = stubs(dir).join("lefthook");
    let write = "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"";
    let out = Command::new("sh")
        .args(["-c", write, "sh", script])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Every file under `dir`, the git directory included, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut left = vec![dir.to_path_buf()];
    while let Some(next) = left.pop() {
        for entry in std::fs::read_dir(&next).unwrap().map(Result::unwrap) {
            let path = entry.path();
            if path.is_dir() {
                left.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap_or_default()
}

const README: &[(&str, &str)] = &[("README.md", "# a project\n")];

/// One runner whose command is a stub: it notes the files it was given.
const STUB_RUNNER: &str = r#"fairlead = "0.19"

[tests]
match = { replace = ["**/*.test.ts"] }

[[tests.runners]]
id = "vitest"
match = ["**/*.test.ts"]
command = ["sh", "run-tests.sh", "{files}"]
"#;

fn with_runner(name: &str, config: &str, script: &str) -> PathBuf {
    repo(
        name,
        &[
            ("fairlead.toml", config),
            ("run-tests.sh", script),
            ("src/a.ts", "export const a = 1;\n"),
            (
                "src/a.test.ts",
                "import { a } from './a';\n// the longer of the two\n",
            ),
            ("src/b.test.ts", "import './a';\n"),
        ],
    )
}

#[cfg(unix)]
#[test]
fn a_repository_with_only_a_readme_gets_the_guard_and_the_hooks_and_nothing_required_stays_open() {
    let dir = repo("readme", README);
    lefthook(&dir, true);
    let (code, out, err) = setup(&dir, &[]);
    assert_eq!(code, 0, "{out}{err}");
    let expected = format!(
        "fairlead setup    track: new (no tests yet, 1 commit)
  ok     binary     {VERSION} on PATH
  fixed  config     fairlead.toml, no problems, guard profile
  fixed  hooks      Claude Code: edit guard, brief nudge, stop gate, session start
  fixed  git-hook   the commit stage is in lefthook.yml, and lefthook runs it
  fixed  agents     the block is current in AGENTS.md, CLAUDE.md
  open   gate       no passing `fairlead done` on record
         fix: fairlead done
  ok     tests      no tests a runner here can run yet
  later  ci         no workflow runs `fairlead guard check`; the new track leaves it for later
1 open. Fix it and run `fairlead setup` again. `fairlead setup --check` shows this list any time.
"
    );
    assert_eq!(out, expected, "{err}");
    assert!(read(&dir, "fairlead.toml").contains("profile = \"guard\""));
    assert!(read(&dir, ".claude/settings.json").contains("fairlead guard hook"));
    assert!(read(&dir, "lefthook.yml").contains("fairlead guard check --staged"));
    assert!(read(&dir, "AGENTS.md").contains("fairlead:begin"));
    assert!(
        !dir.join(".github").exists(),
        "setup never writes a workflow"
    );
    let (_, json, _) = setup(&dir, &["--check", "--json"]);
    let list: Value = serde_json::from_str(&json).unwrap();
    for item in list["items"].as_array().unwrap() {
        assert!(
            item["state"] != "open" || item["required"] == false,
            "{item}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_second_run_fixes_nothing_more_and_prints_no_fixed_line() {
    let dir = repo("twice", README);
    lefthook(&dir, true);
    let (code, first, _) = setup(&dir, &[]);
    assert_eq!(code, 0, "{first}");
    assert!(first.contains("\n  fixed  "), "{first}");
    let before = snapshot(&dir);
    let (code, second, _) = setup(&dir, &[]);
    assert_eq!(code, 0, "{second}");
    assert!(!second.contains("fixed"), "{second}");
    assert_eq!(second, first.replace("  fixed  ", "  ok     "));
    assert!(before == snapshot(&dir), "the second run wrote something");
}

#[cfg(unix)]
#[test]
fn a_runner_is_proved_on_its_smallest_test_file_and_the_mvp_track_shows_ci_open_but_not_required() {
    let script = "echo \"$@\" >> ran.log\necho '1 passed'\n";
    let dir = with_runner("proved", STUB_RUNNER, script);
    lefthook(&dir, true);
    let (code, out, err) = setup(&dir, &[]);
    assert_eq!(code, 0, "an open `ci` doesn't fail an mvp: {out}{err}");
    assert!(
        out.starts_with("fairlead setup    track: mvp (2 test files, no workflow that tests or deploys, under a day of history)\n"),
        "{out}"
    );
    assert!(
        out.contains("\n  fixed  proved     1 runner ran one test file: vitest 0."),
        "{out}"
    );
    assert!(
        out.contains("\n  open   ci         no workflow runs `fairlead ci`\n         fix: fairlead ci workflow --write\n"),
        "{out}"
    );
    assert!(
        out.contains("\n  ok     smoke      no runner has over 50 test files or a need\n"),
        "{out}"
    );
    assert_eq!(read(&dir, "ran.log"), "src/b.test.ts\n");
    let record: Value = serde_json::from_str(&read(&dir, ".git/fairlead/setup.json")).unwrap();
    let proof = &record["proved"][0];
    assert_eq!(proof["runner"], "vitest");
    assert_eq!(proof["file"], "src/b.test.ts");
    assert_eq!(proof["passed"], true);
    assert!(proof["seconds"].is_number() && proof["why"].is_null());
    assert_eq!(read(&dir, "fairlead.toml"), STUB_RUNNER);
    let (_, json, _) = setup(&dir, &["--check", "--json"]);
    let list: Value = serde_json::from_str(&json).unwrap();
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "binary", "config", "runners", "hooks", "git-hook", "agents", "proved", "ci", "smoke",
            "gate", "measure"
        ]
    );
    // The result stands: a later run reads it and runs nothing.
    let (_, again, _) = setup(&dir, &[]);
    assert!(
        again.contains("\n  ok     proved     1 runner ran"),
        "{again}"
    );
    assert_eq!(read(&dir, "ran.log"), "src/b.test.ts\n");
    // The same repository as an ongoing project has to have its workflow.
    let (code, ongoing, _) = setup(&dir, &["--check", "--track", "ongoing"]);
    assert_eq!(code, 1, "{ongoing}");
}

#[cfg(unix)]
#[test]
fn a_failed_test_is_open_with_why_and_a_runner_that_cannot_start_here_is_later() {
    let failing = "echo '1 failed'\necho 'error: expected 1 to be 2' >&2\nexit 1\n";
    let dir = with_runner("failed", STUB_RUNNER, failing);
    lefthook(&dir, true);
    let (code, out, _) = setup(&dir, &[]);
    assert_eq!(code, 0, "`proved` isn't required: {out}");
    assert!(
        out.contains("\n  open   proved     vitest failed on src/b.test.ts: ")
            && out.contains("error: expected 1 to be 2\n         fix: fix the failing test or the runner's command, then `fairlead setup`\n"),
        "{out}"
    );

    let missing = STUB_RUNNER.replace("\"sh\", \"run-tests.sh\"", "\"no-such-tool-x\"");
    let dir = with_runner("missing", &missing, "");
    lefthook(&dir, true);
    let (code, out, _) = setup(&dir, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(
            "\n  later  proved     vitest can't run here: `no-such-tool-x` isn't installed\n"
        ),
        "{out}"
    );

    let needy = format!(
        "{STUB_RUNNER}\n[[needs]]\nid = \"browser\"\nenv = \"SETUP_TEST_BROWSER_UNSET\"\ntests = [\"**/*.test.ts\"]\n"
    );
    let dir = with_runner("need", &needy, "echo ran >> ran.log\n");
    lefthook(&dir, true);
    let (_, out, _) = setup(&dir, &[]);
    assert!(
        out.contains("\n  later  proved     vitest can't run here: need \"browser\" not met\n"),
        "{out}"
    );
    assert_eq!(
        read(&dir, "ran.log"),
        "",
        "nothing ran behind the unmet need"
    );
    assert!(
        out.contains(
            "\n  open   smoke      vitest: need \"browser\" covers it, and it names no `smoke`\n"
        ),
        "{out}"
    );
}

#[cfg(unix)]
#[test]
fn a_git_hook_lefthook_cannot_activate_stays_open_and_fails_the_run() {
    let dir = repo("inactive", README);
    lefthook(&dir, false);
    let (code, out, _) = setup(&dir, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("\n  open   git-hook   written but not active: lefthook.yml has the commit stage, .git/hooks doesn't run lefthook; setup couldn't fix it: `lefthook install` failed: lefthook: no permission\n         fix: lefthook install\n"),
        "{out}"
    );
}

#[test]
fn the_guard_profile_lists_tests_in_place_of_the_runner_items_and_opens_it_once_tests_appear() {
    let guard = "fairlead = \"0.19\"\nprofile = \"guard\"\n";
    let dir = repo(
        "guard",
        &[
            ("fairlead.toml", guard),
            ("package.json", r#"{"devDependencies": {"vitest": "^3"}}"#),
            ("src/a.test.ts", "import './a';\n"),
        ],
    );
    let (_, json, err) = setup(&dir, &["--check", "--json"]);
    let list: Value = serde_json::from_str(&json).expect(&err);
    let items = list["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        ["binary", "config", "hooks", "git-hook", "agents", "ci", "gate", "tests"]
    );
    let tests = items.last().unwrap();
    assert_eq!(tests["state"], "open");
    assert_eq!(tests["required"], false);
    assert_eq!(
        tests["text"],
        "this repository has tests `vitest` can run, and the guard-only profile plans none"
    );
    assert!(
        tests["fix"]
            .as_str()
            .unwrap()
            .starts_with("`fairlead init --dry-run` prints the runner"),
        "{tests}"
    );
}

#[test]
fn hooks_go_in_for_each_agent_whose_folder_is_there() {
    let dir = repo(
        "agents",
        &[("README.md", "x\n"), (".codex/notes.md", "x\n")],
    );
    let (_, out, _) = setup(&dir, &["--check"]);
    assert!(
        out.contains("\n  open   hooks      no hooks for Claude Code, Codex\n         fix: fairlead hooks install\n"),
        "{out}"
    );
    let (_, out, err) = setup(&dir, &[]);
    assert!(
        out.contains("\n  fixed  hooks      Claude Code: edit guard, brief nudge, stop gate, session start; Codex: edit guard, brief nudge, stop gate, session start\n"),
        "{out}{err}"
    );
    assert!(read(&dir, ".codex/hooks.json").contains("fairlead guard hook"));
    assert!(!dir.join(".gemini").exists());
}

#[test]
fn check_and_dry_run_change_nothing_on_disk_and_exit_1_while_a_required_item_is_open() {
    let dir = repo("check", README);
    let before = snapshot(&dir);
    let (code, out, err) = setup(&dir, &["--check"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        out.contains(
            "\n  open   config     no fairlead.toml; defaults apply\n         fix: fairlead init\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("\n  open   hooks      no hooks for Claude Code\n         fix: fairlead hooks install\n"),
        "{out}"
    );
    assert!(out.ends_with("Fix them and run `fairlead setup` again. `fairlead setup --check` shows this list any time.\n"), "{out}");
    assert!(before == snapshot(&dir), "--check wrote something");

    let (code, out, _) = setup(&dir, &["--dry-run"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.starts_with("fairlead setup --dry-run    track: new (no tests yet, 1 commit)\n  would  config     write fairlead.toml with the guard only: init finds no test runner here\n  would  hooks      install the hooks for Claude Code\n"),
        "{out}"
    );
    assert!(
        out.ends_with("Nothing was changed. `fairlead setup` applies these.\n"),
        "{out}"
    );
    assert!(before == snapshot(&dir), "--dry-run wrote something");
}

#[test]
fn json_has_the_track_each_item_and_the_open_count() {
    let dir = repo("json", README);
    let (code, out, err) = setup(&dir, &["--check", "--json"]);
    assert_eq!(code, 1, "{out}{err}");
    let list: Value = serde_json::from_str(&out).unwrap();
    let keys = |v: &Value| -> Vec<String> { v.as_object().unwrap().keys().cloned().collect() };
    assert_eq!(keys(&list), ["track", "items", "open"]);
    assert_eq!(list["track"]["name"], "new");
    assert_eq!(list["track"]["why"], "no tests yet, 1 commit");
    let items = list["items"].as_array().unwrap();
    for item in items {
        assert_eq!(keys(item), ["id", "state", "text", "fix", "required"]);
        assert_eq!(
            item["fix"].is_string(),
            item["state"] == "open",
            "only an open item has a fix: {item}"
        );
    }
    let open = items.iter().filter(|i| i["state"] == "open").count();
    assert_eq!(list["open"], open);
    let config = items.iter().find(|i| i["id"] == "config").unwrap();
    assert_eq!(config["state"], "open");
    assert_eq!(config["fix"], "fairlead init");
    assert_eq!(config["required"], true);
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        ["binary", "config", "hooks", "git-hook", "agents", "gate", "ci"]
    );
}

#[test]
fn track_overrides_the_detected_one_and_the_header_says_why_that_one_was_detected() {
    let dir = repo("track", README);
    let (_, detected, _) = setup(&dir, &["--check"]);
    assert!(
        detected.starts_with("fairlead setup    track: new (no tests yet, 1 commit)\n"),
        "{detected}"
    );
    assert!(detected.contains("\n  later  ci  "), "{detected}");
    let (code, asked, _) = setup(&dir, &["--check", "--track", "ongoing"]);
    assert_eq!(code, 1, "{asked}");
    assert!(
        asked.starts_with("fairlead setup    track: ongoing (asked for with --track; detected new: no tests yet, 1 commit)\n"),
        "{asked}"
    );
    assert!(
        asked.contains("\n  open   ci         no workflow runs `fairlead ci`\n         fix: fairlead ci workflow --write\n"),
        "{asked}"
    );
    let (_, json, _) = setup(&dir, &["--check", "--json", "--track", "ongoing"]);
    let list: Value = serde_json::from_str(&json).unwrap();
    let ci = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "ci")
        .unwrap();
    assert_eq!(ci["required"], true);
    let (code, _, err) = setup(&dir, &["--track", "enterprise"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("new, mvp, ongoing, legacy"), "{err}");
}

#[test]
fn outside_a_git_repository_setup_says_so_and_writes_nothing() {
    let dir = std::env::temp_dir().join(format!("fairlead-setup-nogit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (code, out, err) = setup(&dir, &[]);
    assert_eq!((code, out.as_str()), (2, ""), "{err}");
    assert!(err.contains("not inside a git repository"), "{err}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn doctor_ends_with_how_many_setup_items_are_open_or_that_none_is() {
    let dir = repo("doctor", README);
    lefthook(&dir, true);
    let doctor = || {
        let out = command(&dir, env!("CARGO_BIN_EXE_fairlead"), &["doctor"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let last = |text: &str| text.lines().last().unwrap_or_default().to_string();
    assert_eq!(
        last(&doctor()),
        "setup: 5 open (first: config); `fairlead setup --check` lists each with its fix"
    );
    setup(&dir, &[]);
    assert_eq!(
        last(&doctor()),
        "setup: 1 open (first: gate); `fairlead setup --check` lists each with its fix"
    );
    let now = fairlead_guard::events::timestamp(std::time::SystemTime::now());
    std::fs::write(
        dir.join(".git/fairlead/events.jsonl"),
        format!("{{\"at\":\"{now}\",\"stage\":\"done\",\"decision\":\"pass\",\"rules\":[],\"added\":0,\"ms\":1.0,\"fairlead\":\"test\"}}\n"),
    )
    .unwrap();
    assert_eq!(last(&doctor()), "setup: nothing open");
}

const STATES: [&str; 4] = ["ok", "fixed", "open", "later"];
const IDS: [&str; 12] = [
    "binary", "config", "runners", "proved", "hooks", "git-hook", "agents", "ci", "smoke", "gate",
    "measure", "tests",
];

/// Each ```text block of a page that holds checklist rows.
fn samples(page: &str) -> Vec<Vec<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(page);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{page}: {e}"));
    let mut out = Vec::new();
    let mut block: Option<Vec<String>> = None;
    for line in text.lines() {
        match (line.starts_with("```"), block.take()) {
            (true, Some(lines)) => out.push(lines),
            (true, None) if line == "```text" => block = Some(Vec::new()),
            (false, Some(mut lines)) => {
                lines.push(line.to_string());
                block = Some(lines);
            }
            _ => {}
        }
    }
    let row = |l: &String| STATES.iter().any(|s| l.starts_with(&format!("  {s} ")));
    out.retain(|lines| lines.iter().any(row));
    out
}

#[test]
fn every_checklist_the_docs_show_has_the_shape_setup_prints() {
    let pages = [
        "../../docs/src/start.md",
        "../../docs/src/troubleshooting.md",
        "guides/setup.md",
    ];
    let mut seen = 0;
    for page in pages {
        for block in samples(page) {
            seen += 1;
            let mut open = 0;
            for (at, line) in block.iter().enumerate() {
                let words: Vec<&str> = line.split_whitespace().collect();
                if line.starts_with("         fix: ") || !line.starts_with("  ") {
                    continue;
                }
                assert!(STATES.contains(&words[0]), "{page}: state in `{line}`");
                assert!(IDS.contains(&words[1]), "{page}: id in `{line}`");
                assert_eq!(&line[..20], format!("  {:<7}{:<11}", words[0], words[1]));
                let fixed = (block.get(at + 1)).is_some_and(|l| l.starts_with("         fix: "));
                assert_eq!(words[0] == "open", fixed, "{page}: fix under `{line}`");
                open += usize::from(words[0] == "open");
            }
            let Some(last) = block.iter().find(|l| l.contains(" open. Fix ")) else {
                continue;
            };
            let said = match open {
                1 => "1 open. Fix it and".to_string(),
                n => format!("{n} open. Fix them and"),
            };
            assert!(
                last.starts_with(&said),
                "{page}: `{last}` after {open} open"
            );
        }
    }
    assert!(seen >= 16, "{seen} checklists found in the docs");
}

#[cfg(unix)]
#[test]
fn the_new_track_sample_in_start_here_is_what_setup_prints_on_such_a_repository() {
    let dir = repo("docsnew", README);
    for n in 1..6 {
        std::fs::write(dir.join("README.md"), format!("# a project\n{n}\n")).unwrap();
        let out = command(&dir, "git", &["commit", "-qam", "note"]).output();
        assert!(out.unwrap().status.success());
    }
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    lefthook(&dir, true);
    let (code, out, err) = setup(&dir, &[]);
    assert_eq!(code, 0, "{out}{err}");
    let shown = samples("../../docs/src/start.md")
        .into_iter()
        .find(|b| b[0].contains("track: new "))
        .expect("the new track's sample");
    // The docs name the release they ship with; the binary names itself.
    let binary = |l: &str| l.starts_with("  ok     binary ");
    let printed: Vec<&str> = out.lines().filter(|l| !binary(l)).collect();
    let shown: Vec<&str> = (shown.iter().map(String::as_str))
        .filter(|l| !binary(l))
        .collect();
    assert_eq!(printed, shown);
    std::fs::remove_dir_all(&dir).unwrap();
}
