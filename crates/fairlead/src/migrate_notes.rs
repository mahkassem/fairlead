//! What each release changed that `fairlead migrate` can't change for you,
//! and the table of config keys by the release that added them. Every
//! release in CHANGELOG.md has an entry here, empty when nothing needs a person.

use std::path::Path;

use fairlead_core::config::Loaded;
use fairlead_core::pattern::Pattern;

use crate::test_bench::{is_bun_test, Mode};

pub struct Context<'a> {
    pub loaded: &'a Loaded,
    pub root: &'a Path,
    /// Whether Claude Code hooks from Fairlead are in the repository.
    pub claude_hooks: bool,
}

pub struct Note {
    pub text: &'static str,
    /// Whether the note concerns this repository; notes that can't tell say yes.
    pub applies: fn(&Context) -> bool,
}

fn always(_: &Context) -> bool {
    true
}

fn go_manifests_in_run_all(cx: &Context) -> bool {
    cx.loaded
        .config
        .plan
        .run_all
        .items()
        .iter()
        .flat_map(|entry| entry.globs())
        .any(|p| p.contains("go.mod") || p.contains("go.sum"))
}

fn has_claude_hooks(cx: &Context) -> bool {
    cx.claude_hooks
}

/// Whether a layer other than the built-in defaults sets a key under `prefix`.
fn set(cx: &Context, prefix: &str) -> bool {
    cx.loaded
        .origins
        .iter()
        .any(|(key, layer)| key.starts_with(prefix) && layer != "default")
}

fn has_owners(cx: &Context) -> bool {
    !cx.loaded.config.tests.owners.items().is_empty()
}

fn has_empty_list_override(cx: &Context) -> bool {
    cx.loaded
        .warnings
        .iter()
        .any(|w| w.message.contains("replace = []"))
}

fn uses_replay(cx: &Context) -> bool {
    set(cx, "replay")
}

fn has_next(cx: &Context) -> bool {
    let manifest: serde_json::Value = std::fs::read_to_string(cx.root.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    ["dependencies", "devDependencies"]
        .iter()
        .any(|k| manifest[k].get("next").is_some())
}

fn several_runners(cx: &Context) -> bool {
    cx.loaded.config.tests.runners.items().len() > 1
}

fn extends_next(cx: &Context) -> bool {
    cx.loaded
        .origins
        .values()
        .any(|layer| layer.ends_with("pack next"))
}

fn run_all_covers_a_barrier(cx: &Context) -> bool {
    let config = &cx.loaded.config;
    names_only_barriers(config.plan.run_all.items(), config.graph.barrier.items())
}

/// Whether a `run_all` entry is a barrier entry itself, or a plain path a
/// barrier glob matches: one that covered only a barrier's own changes.
fn names_only_barriers(run_all: &[fairlead_core::config::RunAll], barrier: &[String]) -> bool {
    let globs: Vec<Pattern> = barrier
        .iter()
        .filter_map(|g| Pattern::new(g).ok())
        .collect();
    run_all.iter().flat_map(|entry| entry.globs()).any(|entry| {
        barrier.contains(entry)
            || (!entry.contains(['*', '?', '[', '{']) && globs.iter().any(|g| g.is_match(entry)))
    })
}

fn has_go(cx: &Context) -> bool {
    cx.root.join("go.mod").is_file() || cx.root.join("go.work").is_file()
}

fn has_jvm_build(cx: &Context) -> bool {
    [
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
    ]
    .iter()
    .any(|f| cx.root.join(f).is_file())
}

/// Whether a runner still runs `bun test` one file after another; whether a
/// parallel mode is faster depends on the suite, so the bench decides.
fn serial_bun_test(cx: &Context) -> bool {
    cx.loaded
        .config
        .tests
        .runners
        .items()
        .iter()
        .any(|r| is_bun_test(&r.command) && Mode::of(&r.command) == Mode::Serial)
}

/// Each release, oldest first, with what it changed that needs a decision.
pub const RELEASES: &[(&str, &[Note])] = &[
    ("0.1.0", &[]),
    ("0.1.1", &[]),
    ("0.2.0", &[]),
    ("0.3.0", &[]),
    (
        "0.4.0",
        &[Note {
            text: "`[[guard.external]]` refuses `stages = [\"write\"]`, since the file isn't written yet when the write hook runs. Use `commit` or `ci` for such a rule.",
            applies: always,
        }],
    ),
    ("0.4.2", &[]),
    (
        "0.5.0",
        &[
            Note {
                text: "`plan.ignore` applies to source files too: a changed source nothing imports that matches it selects nothing, where it used to widen to every test. Under the defaults that's `docs/**`. Keep sources whose tests you want run out of `plan.ignore`.",
                applies: always,
            },
            Note {
                text: "An owner rule whose `match` names no test file no longer counts as covering its paths: they fall to `tests.unreached`. `config check` warns about each one. Make a rule meant to select nothing a `plan.ignore` entry.",
                applies: has_owners,
            },
            Note {
                text: "`key = []` over a list that already has items changes nothing, since lists append; `{ replace = [] }` clears it. `config check` warns where a layer does this.",
                applies: has_empty_list_override,
            },
            Note {
                text: "A Go module's `go.mod` and `go.sum` are graph edges instead of `plan.run_all` defaults, so a dependency bump selects that module and its importers. Your `plan.run_all` still lists them, which keeps running everything on them; drop them to get the narrower plan.",
                applies: go_manifests_in_run_all,
            },
            Note {
                text: "Go and Python files are in the import graph. A Go or Python repository that ran everything through a `[[checks]]` entry can give its tests a runner instead (see Import graph).",
                applies: has_go,
            },
            Note {
                text: "Replay datasets fetched before 0.5.0 keep no lines for a bun test file that failed to load; fetch them again to count those failures.",
                applies: uses_replay,
            },
        ],
    ),
    (
        "0.5.1",
        &[Note {
            text: "A changed path matching `plan.ignore` that reaches no test is ignored even when other files import it, where 0.5.0 sent it to `tests.unreached`. Plans for such changes get smaller.",
            applies: always,
        }],
    ),
    (
        "0.6.0",
        &[Note {
            text: "`hooks install` adds a Stop hook, which sends an agent back while `fairlead done` hasn't passed, and a nudge after an edit made with no brief. `migrate --write` adds both to hooks installed before; set `done.on_stop = \"off\"` or `brief.nudge = false` first to leave one out.",
            applies: has_claude_hooks,
        }],
    ),
    (
        "0.7.0",
        &[Note {
            text: "Java and Kotlin files are in the import graph. A Maven or Gradle build that ran everything through a `[[checks]]` entry can give its tests a runner instead, with `{class}` or `{classes}` in the command (see Java and Kotlin).",
            applies: has_jvm_build,
        }],
    ),
    (
        "0.7.1",
        &[Note {
            text: "When everything runs, a runner's command runs with no files, so a tool that finds its own tests can run more than the runner's `match` claims. `all_command` gives the command for that case, and `exclude_arg` keeps it whole around a test `[[quarantine]]` holds (see Configuration).",
            applies: always,
        }],
    ),
    (
        "0.8.0",
        &[
            Note {
                text: "`hooks install` adds `Skill` to the edit hook's matcher, so loading a skill counts as a use, and a SessionStart hook that runs `fairlead resume --hook`. `migrate --write` brings both to hooks installed before; set `brief.resume = false` first to leave out the SessionStart hook.",
                applies: has_claude_hooks,
            },
            Note {
                text: "Lessons, skill routes and the AGENTS.md block are new and stay off until a repository adds them. `fairlead import lessons` and `import rules` start from a lessons document and rule files a team already keeps (see Lessons and Skills).",
                applies: always,
            },
            Note {
                text: "CI stages stay off until the config sets `[stages]` or a step's `from`. Then `ci plan` reads the stage from the GitHub event, and `fairlead ci workflow` writes a staged workflow, or prints the `if:` lines for one you keep (see Plans in CI). A bun dependency's version bump, a Fairlead config edit and a workflow that only runs by hand or on a schedule no longer select every test.",
                applies: always,
            },
        ],
    ),
    (
        "0.9.0",
        &[
            Note {
                text: "`replay fetch --event schedule` records the default branch's scheduled runs, so a project whose pushes run the plan and whose full suite runs on a schedule can count what the nightly caught and no push ran (see Replay).",
                applies: uses_replay,
            },
            Note {
                text: "`fairlead graph why` names edge kinds in words (`type import`, `path literal`) where it printed `typeimport` and `pathliteral`; a script that reads its output needs the new names.",
                applies: always,
            },
        ],
    ),
    (
        "0.10.0",
        &[
            Note {
                text: "Replay sets recurring failures aside: a test failing in one job across three or more unrelated pull requests leaves adjusted recall, so a gate can pass that failed before; confirm the `[[replay.quarantine]]` entries it lists (see Replay).",
                applies: uses_replay,
            },
            Note {
                text: "`fairlead doctor --score` rates the setup out of 100, and `fairlead ci workflow --insights --write` adds a weekly summary of how it works (see Readiness score, Insights).",
                applies: always,
            },
        ],
    ),
    (
        "0.11.0",
        &[
            Note {
                text: "Sessions in a linked worktree count now: every worktree writes to the main checkout's `.git/fairlead/events.jsonl`, so `doctor --score`, `insights` and `tokens report` may show more than before (see Guard).",
                applies: always,
            },
            Note {
                text: "This is a Next.js app: `extends = [\"next\"]` makes the layouts and `_app` above a route reach the tests under it (see Framework packs).",
                applies: has_next,
            },
            Note {
                text: "Building Fairlead from source needs Rust 1.95; the release binaries and the npm package are unaffected.",
                applies: always,
            },
        ],
    ),
    (
        "0.12.0",
        &[
            Note {
                text: "The `next` pack no longer ties every end-to-end spec that calls `page.goto(` to every route and the middleware: that rule is the `next-e2e` pack now. Add `\"next-e2e\"` to `extends` to keep it, or leave it out when owner rules already map pages to specs (see Framework packs).",
                applies: extends_next,
            },
            Note {
                text: "When everything runs, a runner whose folder holds another runner's tests names its own tests instead of leaving the tool to find them, so its command line lists them; give it `all_command` to keep the tool's own discovery (see Planning).",
                applies: several_runners,
            },
        ],
    ),
    (
        "0.13.0",
        &[Note {
            text: "A changed `graph.barrier` file selects the tests of the files that import it, as any changed file does; a change below it still stops there. A `plan.run_all` entry that covered only a barrier's own changes can go, and leaving it keeps running everything when the barrier changes (see Barriers).",
            applies: run_all_covers_a_barrier,
        }],
    ),
    (
        "0.14.0",
        &[Note {
            text: "A runner runs `bun test` one file after another. Bun 1.4.4 adds `--parallel` and `--parallel --no-isolate`, which can be much faster or slower depending on the suite and tend to turn up order-dependent tests; `fairlead test bench` times the three on this repository's tests and says which to use. The runner is left as it is (see Measuring Bun's test modes).",
            applies: serial_bun_test,
        }],
    ),
];

/// Config tables and keys by the release that added them. The top-level
/// tables older than all of these are `BASE`; a test holds every other
/// top-level table to having an entry here. A key inside a list of tables,
/// such as `tests.runners.all_command`, counts when any item sets it.
pub const SINCE: &[(&str, &str)] = &[
    ("graph.edges", "0.3"),
    ("graph.barrier", "0.3"),
    ("guard", "0.4"),
    ("hooks", "0.4"),
    ("graph.providers", "0.5"),
    ("graph.coverage", "0.5"),
    ("done", "0.6"),
    ("brief", "0.6"),
    ("brief.resume", "0.8"),
    ("ci", "0.6"),
    ("quarantine", "0.7"),
    ("tests.runners.all_command", "0.7.1"),
    ("tests.runners.exclude_arg", "0.7.1"),
    ("memory", "0.8"),
    ("skills", "0.8"),
    ("agents", "0.8"),
    ("stages", "0.8"),
    ("tests.runners.from", "0.8"),
    ("checks.from", "0.8"),
    ("guard.commands.unless", "0.8"),
    ("extends", "0.10"),
    ("graph.edges.find", "0.10"),
    ("workspace", "0.10"),
    ("tracker", "0.10"),
    ("workspace.edges", "0.11"),
    ("plan.run_all.on", "0.13"),
];

/// Top-level tables every released config format has had.
#[cfg(test)]
pub const BASE: &[&str] = &[
    "fairlead", "modules", "graph", "tests", "checks", "plan", "replay",
];

#[cfg(test)]
mod tests {
    use super::{names_only_barriers, serial_bun_test, Context};
    use fairlead_core::config::RunAll;

    #[test]
    fn the_bench_note_is_for_a_bun_test_runner_with_no_parallel_mode() {
        let applies = |command: &str| {
            let text =
                format!("[[tests.runners]]\nid = \"t\"\nmatch = [\"**\"]\ncommand = {command}\n");
            let dir =
                std::env::temp_dir().join(format!("fairlead-notes-bench-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("fairlead.toml"), text).unwrap();
            let loaded = fairlead_core::config::load(&dir, &Default::default()).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            serial_bun_test(&Context {
                loaded: &loaded,
                root: &dir,
                claude_hooks: false,
            })
        };
        assert!(applies(r#"["bun", "test", "{files}"]"#));
        assert!(!applies(r#"["bun", "test", "--parallel", "{files}"]"#));
        assert!(!applies(
            r#"["bun", "test", "--parallel", "--no-isolate", "{files}"]"#
        ));
        assert!(!applies(r#"["vitest", "run", "{files}"]"#));
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn globs(items: &[&str]) -> Vec<RunAll> {
        items.iter().map(|s| RunAll::Glob(s.to_string())).collect()
    }

    #[test]
    fn a_run_all_entry_counts_when_it_is_a_barrier_entry_or_a_path_one_matches() {
        let barrier = strings(&["src/http/**", "src/loader.ts"]);
        assert!(names_only_barriers(&globs(&["src/loader.ts"]), &barrier));
        assert!(names_only_barriers(&globs(&["src/http/**"]), &barrier));
        assert!(names_only_barriers(
            &globs(&["migrations/**", "src/http/server.ts"]),
            &barrier
        ));
        assert!(!names_only_barriers(&globs(&["src/**"]), &barrier));
        assert!(!names_only_barriers(&globs(&["src/db.ts"]), &barrier));
        assert!(!names_only_barriers(&globs(&["src/loader.ts"]), &[]));
        let kinds: Vec<RunAll> = toml::from_str::<toml::Table>(
            "e = { paths = [\"src/loader.ts\"], on = [\"modified\"] }",
        )
        .unwrap()["e"]
            .clone()
            .try_into()
            .map(|e| vec![e])
            .unwrap();
        assert!(
            names_only_barriers(&kinds, &barrier),
            "a kind-limited entry counts too"
        );
    }
}
