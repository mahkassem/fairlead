//! Invocations: each runner's selected tests, and each selected check, as
//! an argv in a working directory. `{files}` expands to one argument per
//! file and `{packages}` to one per directory holding them, as `go test`
//! takes packages (`./dir`, or `./...` for everything); a per-module runner gets
//! one invocation per module, with files relative to its working directory.
//! JVM runners take class names: an argument holding `{class}` is repeated
//! once per class, as Gradle's `--tests={class}`, and `{classes}` joins them
//! with commas, as Surefire's `-Dtest={classes}`; either is dropped when
//! everything runs.

use std::collections::BTreeMap;

use fairlead_core::config::{CheckFiles, Invoke, Runner};
use fairlead_core::plan::{CheckSelection, Invocation, InvocationKind, TestSelection};

use crate::pattern::Pattern;
use crate::planner::Context;

fn relative_to(cwd: &str, path: &str) -> String {
    if cwd == "." || cwd.is_empty() {
        return path.to_string();
    }
    path.strip_prefix(cwd)
        .and_then(|rest| rest.strip_prefix('/'))
        .map_or_else(|| path.to_string(), str::to_string)
}

fn expand(
    command: &[String],
    files: &[String],
    all: bool,
    module: Option<(&str, &str)>,
) -> Vec<String> {
    let mut argv = Vec::new();
    for arg in command {
        if arg == "{files}" {
            argv.extend(files.iter().cloned());
            continue;
        }
        if arg.contains("{class}") || arg.contains("{classes}") {
            if !all {
                let classes: Vec<String> = files.iter().map(|f| class_name(f)).collect();
                if arg.contains("{class}") {
                    argv.extend(classes.iter().map(|c| arg.replace("{class}", c)));
                } else {
                    argv.push(arg.replace("{classes}", &classes.join(",")));
                }
            }
            continue;
        }
        if arg == "{packages}" && all {
            argv.push("./...".to_string());
            continue;
        }
        if arg == "{packages}" {
            let mut dirs: Vec<String> = files
                .iter()
                .map(|f| match f.rsplit_once('/') {
                    Some((dir, _)) => format!("./{dir}"),
                    None => ".".to_string(),
                })
                .collect();
            dirs.sort();
            dirs.dedup();
            argv.extend(dirs);
            continue;
        }
        let arg = match module {
            Some((root, id)) => arg.replace("{module.id}", id).replace("{module}", root),
            None => arg.clone(),
        };
        argv.push(arg);
    }
    argv
}

/// A JVM test file's class, by the source-set layout Maven and Gradle
/// share (`src/test/java/com/acme/FooTest.java` is `com.acme.FooTest`), or
/// its file name outside it.
fn class_name(file: &str) -> String {
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let mut parts = stem.split('/').collect::<Vec<_>>();
    let root = parts
        .windows(3)
        .rposition(|w| w[0] == "src" && matches!(w[2], "java" | "kotlin" | "groovy" | "scala"));
    match root {
        Some(i) => parts.drain(..i + 3),
        None => parts.drain(..parts.len() - 1),
    };
    parts.join(".")
}

fn runner_invocations(
    cx: &Context,
    runner: &Runner,
    tests: &[&TestSelection],
    all: bool,
) -> Vec<Invocation> {
    let invocation = |cwd: String, argv: Vec<String>| Invocation {
        id: runner.id.clone(),
        kind: InvocationKind::Runner,
        cwd,
        argv,
    };
    match runner.invoke {
        Invoke::Once => {
            if tests.is_empty() && !all {
                return Vec::new();
            }
            let cwd = runner
                .cwd
                .clone()
                .filter(|c| !c.contains('{'))
                .unwrap_or_else(|| ".".into());
            let files: Vec<String> = if all {
                Vec::new()
            } else {
                tests.iter().map(|t| relative_to(&cwd, &t.path)).collect()
            };
            vec![invocation(cwd, expand(&runner.command, &files, all, None))]
        }
        Invoke::PerModule => {
            let mut groups: BTreeMap<Option<String>, Vec<&TestSelection>> = BTreeMap::new();
            if all {
                for test in cx.tests.iter().filter(|t| {
                    t.runner.map(|r| &cx.config.tests.runners.items()[r].id) == Some(&runner.id)
                }) {
                    groups
                        .entry(test.module.map(|m| cx.modules.get(m).name.clone()))
                        .or_default();
                }
            }
            for test in tests {
                groups.entry(test.module.clone()).or_default().push(test);
            }
            groups
                .into_iter()
                .map(|(module, tests)| {
                    let root = module
                        .as_ref()
                        .and_then(|name| cx.modules.all().iter().find(|m| &m.name == name))
                        .map_or(".".to_string(), |m| m.root.clone());
                    let id = module.clone().unwrap_or_default();
                    let cwd = runner.cwd.as_deref().map_or(".".to_string(), |c| {
                        c.replace("{module.id}", &id).replace("{module}", &root)
                    });
                    let files: Vec<String> = if all {
                        Vec::new()
                    } else {
                        tests.iter().map(|t| relative_to(&cwd, &t.path)).collect()
                    };
                    invocation(
                        cwd,
                        expand(&runner.command, &files, all, Some((&root, &id))),
                    )
                })
                .collect()
        }
    }
}

fn check_files(
    cx: &Context,
    paths: &[Pattern],
    files: Option<CheckFiles>,
    all: bool,
) -> Vec<String> {
    let matched = |p: &String| paths.iter().any(|g| g.is_match(p));
    if all || files == Some(CheckFiles::Matched) {
        return cx
            .scan
            .tree
            .files
            .iter()
            .filter(|p| matched(p))
            .cloned()
            .collect();
    }
    cx.changed
        .iter()
        .filter(|p| !cx.deleted.contains(*p) && matched(p))
        .cloned()
        .collect()
}

pub fn invocations(
    cx: &Context,
    tests: &[TestSelection],
    checks: &[CheckSelection],
    all: bool,
) -> Vec<Invocation> {
    let mut out = Vec::new();
    for runner in cx.config.tests.runners.items() {
        let mine: Vec<&TestSelection> = tests
            .iter()
            .filter(|t| t.runner.as_deref() == Some(runner.id.as_str()))
            .collect();
        out.extend(runner_invocations(cx, runner, &mine, all));
    }
    for selected in checks {
        let Some(check) = cx
            .config
            .checks
            .items()
            .iter()
            .find(|c| c.id == selected.id)
        else {
            continue;
        };
        let paths: Vec<Pattern> = check
            .paths
            .iter()
            .filter_map(|g| Pattern::new(g).ok())
            .collect();
        let files = if check.command.iter().any(|a| a == "{files}") {
            check_files(cx, &paths, check.files, all)
        } else {
            Vec::new()
        };
        out.push(Invocation {
            id: check.id.clone(),
            kind: InvocationKind::Check,
            cwd: ".".into(),
            argv: expand(&check.command, &files, false, None),
        });
    }
    out
}
