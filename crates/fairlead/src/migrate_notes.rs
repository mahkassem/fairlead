//! What each release changed that `fairlead migrate` can't change for you,
//! and the table of config keys by the release that added them. Every
//! release in CHANGELOG.md has an entry here, empty when nothing needs a person.

use std::path::Path;

use fairlead_core::config::Loaded;

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
];

/// Top-level tables every released config format has had.
#[cfg(test)]
pub const BASE: &[&str] = &[
    "fairlead", "modules", "graph", "tests", "checks", "plan", "replay",
];
