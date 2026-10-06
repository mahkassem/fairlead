//! Changed paths that would select everything but can't change what a test
//! does: a dependency's version moving, which reaches only the files that
//! import it; Fairlead's own config, which the plan already uses; and a
//! workflow that runs only by hand or on a schedule. Each one that applies
//! says so in a warning, and anything uncertain is left as it was.

use std::collections::{BTreeMap, BTreeSet};

use fairlead_core::config::Config;
use fairlead_core::plan::{Change, Status, Warning};
use fairlead_lang::resolve::package_name;
use fairlead_lang::tree::parent;
use fairlead_lang::Scan;
use serde_json::Value;

use crate::bump::{self, BunLock};
use crate::pattern::Pattern;

pub const BUN_LOCK: &str = "bun.lock";

#[derive(Default)]
pub struct Trims {
    /// Changed paths that select nothing by themselves.
    pub quiet: BTreeSet<String>,
    /// Files importing a moved package, each with `<path> (<package>)`
    /// naming the change it stands for.
    pub starts: Vec<(u32, String)>,
    pub warnings: Vec<Warning>,
}

/// Whether the plan reads `path` at the base to decide a trim.
pub fn reads_base(path: &str) -> bool {
    path == BUN_LOCK || path.ends_with("package.json") || is_workflow(path)
}

pub fn trims(
    scan: &Scan,
    config: &Config,
    changes: &[Change],
    base_files: &BTreeMap<String, String>,
    run_all: &[Pattern],
) -> Trims {
    let mut out = Trims::default();
    let head = |path: &str| std::fs::read_to_string(scan.tree.root.join(path)).ok();
    for change in changes {
        let path = change.path.as_str();
        if is_config(path) && !run_all.iter().any(|g| g.is_match(path)) {
            out.quiet.insert(path.to_string());
            out.warnings.push(Warning {
                code: "fairlead-config".into(),
                path: Some(path.to_string()),
                message: "Fairlead's own config changed; the plan is made with the new one, so the file doesn't select everything".into(),
            });
        }
        for (path, at_head, at_base) in sides(change) {
            if !is_workflow(path) {
                continue;
            }
            let reads =
                |want: bool, text: Option<String>| !want || text.is_some_and(|t| dispatch_only(&t));
            if (at_head || at_base)
                && reads(at_head, head(path))
                && reads(at_base, base_files.get(path).cloned())
            {
                out.quiet.insert(path.to_string());
                out.warnings.push(Warning {
                    code: "workflow-dispatch-only".into(),
                    path: Some(path.to_string()),
                    message: "this workflow runs only on workflow_dispatch or schedule, at the base and the head, so it can't change a pull request's or a push's run and doesn't select everything".into(),
                });
            }
        }
    }
    version_bumps(scan, config, changes, base_files, run_all, &mut out);
    out
}

/// The paths a change touches, each with whether it exists at the head and
/// at the base.
fn sides(change: &Change) -> Vec<(&str, bool, bool)> {
    let path = change.path.as_str();
    match (change.status, &change.from) {
        (Status::Added, _) => vec![(path, true, false)],
        (Status::Modified, _) => vec![(path, true, true)],
        (Status::Deleted, _) => vec![(path, false, true)],
        (Status::Renamed, Some(from)) => vec![(path, true, false), (from.as_str(), false, true)],
        (Status::Renamed, None) => vec![(path, true, true)],
    }
}

/// `fairlead.toml` or a layer over it, such as `fairlead.ci.yaml`, at the
/// root where the config is read from.
fn is_config(path: &str) -> bool {
    let Some(stem) = ["toml", "yaml", "yml"]
        .iter()
        .find_map(|ext| path.strip_suffix(&format!(".{ext}")))
    else {
        return false;
    };
    !stem.contains('/')
        && (stem == "fairlead"
            || stem
                .strip_prefix("fairlead.")
                .is_some_and(|l| !l.is_empty()))
}

/// A workflow file: GitHub reads only the folder's top level.
fn is_workflow(path: &str) -> bool {
    path.strip_prefix(".github/workflows/").is_some_and(|name| {
        !name.contains('/') && (name.ends_with(".yml") || name.ends_with(".yaml"))
    })
}

/// Whether a workflow's only triggers are `workflow_dispatch` and
/// `schedule`. Anything else, `workflow_call` and `workflow_run` included,
/// can run for a pull request or a push.
pub fn dispatch_only(text: &str) -> bool {
    let Ok(Value::Object(doc)) = serde_saphyr::from_str::<Value>(text) else {
        return false;
    };
    let Some(on) = doc.get("on").or_else(|| doc.get("true")) else {
        return false;
    };
    let names: Vec<&str> = match on {
        Value::String(s) => vec![s.as_str()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        Value::Object(map) => map.keys().map(String::as_str).collect(),
        _ => return false,
    };
    let count = match on {
        Value::Array(items) => items.len(),
        _ => names.len(),
    };
    count > 0
        && names.len() == count
        && names
            .iter()
            .all(|n| matches!(*n, "workflow_dispatch" | "schedule"))
}

/// A root or workspace `package.json`, or the root `bun.lock`, whose change
/// only moves dependency versions: the files importing a moved package, or
/// a package depending on one, start the walk instead of everything.
fn version_bumps(
    scan: &Scan,
    config: &Config,
    changes: &[Change],
    base_files: &BTreeMap<String, String>,
    run_all: &[Pattern],
    out: &mut Trims,
) {
    let read = |path: &str| std::fs::read_to_string(scan.tree.root.join(path)).ok();
    let Some(head_lock) = read(BUN_LOCK).and_then(|t| BunLock::parse(&t)) else {
        return;
    };
    let mut base_lock = None;
    let mut moved: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for change in changes.iter().filter(|c| c.status == Status::Modified) {
        let path = change.path.as_str();
        let manifest = path == "package.json"
            || (path.ends_with("/package.json")
                && scan.packages.iter().any(|p| p.dir == parent(path)));
        let Some(base) = base_files
            .get(path)
            .filter(|_| manifest || path == BUN_LOCK)
        else {
            continue;
        };
        let names = if path == BUN_LOCK {
            base_lock = BunLock::parse(base);
            base_lock
                .as_ref()
                .and_then(|b| bump::lock_bumps(b, &head_lock))
        } else {
            read(path).and_then(|head| bump::manifest_bumps(base, &head))
        };
        if let Some(names) = names {
            moved.insert(path.to_string(), names);
        }
    }
    if moved.is_empty() {
        return;
    }
    // A manifest names the packages its own author moved, so it labels them
    // before the lockfile does.
    let mut seeds = BTreeMap::new();
    for (path, names) in moved
        .iter()
        .filter(|(p, _)| *p != BUN_LOCK)
        .chain(moved.get_key_value(BUN_LOCK))
    {
        for name in names {
            seeds.entry(name.clone()).or_insert_with(|| path.clone());
        }
    }
    let locks: Vec<&BunLock> = [Some(&head_lock), base_lock.as_ref()]
        .into_iter()
        .flatten()
        .collect();
    let affected = bump::dependents(&locks, seeds);
    if let Some(warning) = named_by_a_runner(config, &locks, &affected) {
        out.warnings.push(warning);
        return;
    }
    let (mut hit, mut starts) = (BTreeSet::new(), Vec::new());
    for (file, name) in imports(scan, &affected) {
        let path = &scan.graph.files[file as usize];
        let source = &affected[name];
        if run_all.iter().any(|g| g.is_match(path)) {
            out.warnings.push(Warning {
                code: "version-bump-runs-everything".into(),
                path: Some(source.clone()),
                message: format!("`{path}` imports `{name}`, whose version moved, and it selects everything when it changes"),
            });
            return;
        }
        hit.insert(name);
        starts.push((file, format!("{source} ({name})")));
    }
    for (path, names) in &moved {
        let own = names.iter().map(|n| (n.clone(), path.clone())).collect();
        let reached = bump::dependents(&locks, own)
            .keys()
            .any(|n| hit.contains(n.as_str()));
        out.warnings.push(Warning {
            code: "version-bump-scoped".into(),
            path: Some(path.clone()),
            message: bump_message(names, reached),
        });
    }
    out.quiet.extend(moved.into_keys());
    out.starts = starts;
}

/// Every file importing one of `names` from outside the repository, whether
/// the package is installed here (resolved into `node_modules`) or not.
fn imports<'a>(
    scan: &'a Scan,
    names: &'a BTreeMap<String, String>,
) -> impl Iterator<Item = (u32, &'a str)> + 'a {
    let graph = &scan.graph;
    let failed = graph
        .failed
        .iter()
        .filter_map(|(f, spec, _)| package_name(spec).map(|n| (*f, n)));
    graph
        .external
        .iter()
        .map(|(f, n)| (*f, n.as_str()))
        .chain(failed)
        .filter(|(_, n)| names.contains_key(*n))
}

fn bump_message(names: &BTreeSet<String>, reached: bool) -> String {
    if names.is_empty() {
        return "only the package's own version moved, so the file selects nothing by itself"
            .into();
    }
    let mut list: Vec<String> = names.iter().take(5).map(|n| format!("`{n}`")).collect();
    if names.len() > 5 {
        list.push(format!("{} more", names.len() - 5));
    }
    let then = if reached {
        "the files that import them, or a package that depends on them, start the walk instead of everything"
    } else {
        "nothing here imports them or a package that depends on them, so the file selects nothing by itself"
    };
    format!(
        "only dependency versions moved ({}); {then}",
        list.join(", ")
    )
}

/// A moved package a test runner's command names, by package or by one of
/// the commands it installs: it changes how every test of that runner runs.
fn named_by_a_runner(
    config: &Config,
    locks: &[&BunLock],
    affected: &BTreeMap<String, String>,
) -> Option<Warning> {
    for runner in config.tests.runners.items() {
        let argv = runner
            .command
            .iter()
            .chain(runner.all_command.iter().flatten())
            .chain(runner.exclude_arg.iter().flatten());
        let words: BTreeSet<&str> = argv
            .flat_map(|arg| arg.split_whitespace())
            .flat_map(|word| [word, word.rsplit('/').next().unwrap_or(word)])
            .collect();
        for (name, source) in affected {
            let bins: BTreeSet<String> = locks.iter().flat_map(|l| l.bins(name)).collect();
            if words.contains(name.as_str()) || bins.iter().any(|b| words.contains(b.as_str())) {
                return Some(Warning {
                    code: "version-bump-runs-everything".into(),
                    path: Some(source.clone()),
                    message: format!(
                        "`{name}`, whose version moved, is how runner `{}` runs its tests",
                        runner.id
                    ),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workflow_run_only_by_hand_or_on_a_schedule_is_dispatch_only() {
        for on in [
            "on: workflow_dispatch",
            "on: [workflow_dispatch, schedule]",
            "on:\n  workflow_dispatch:\n    inputs:\n      x:\n        type: string\n  schedule:\n    - cron: '0 3 * * *'",
            "\"on\":\n  schedule:\n    - cron: '0 3 * * *'",
        ] {
            assert!(dispatch_only(&format!("name: n\n{on}\njobs: {{}}\n")), "{on}");
        }
    }

    #[test]
    fn a_workflow_with_any_other_trigger_is_not() {
        for on in [
            "on: push",
            "on: [workflow_dispatch, pull_request]",
            "on:\n  workflow_dispatch:\n  workflow_call:",
            "on:\n  schedule:\n    - cron: '0 3 * * *'\n  workflow_run:\n    workflows: [ci]",
            "on: []",
            "on: {}",
            "on: [workflow_dispatch, {a: b}]",
            "jobs: {}",
            "on: [workflow_dispatch",
        ] {
            assert!(!dispatch_only(&format!("name: n\n{on}\n")), "{on}");
        }
    }

    #[test]
    fn config_files_are_the_project_file_and_its_layers_at_the_root() {
        for path in [
            "fairlead.toml",
            "fairlead.yaml",
            "fairlead.yml",
            "fairlead.ci.toml",
            "fairlead.local.yaml",
        ] {
            assert!(is_config(path), "{path}");
        }
        for path in [
            "fairlead.json",
            "sub/fairlead.toml",
            "fairlead..toml",
            "fairleads.toml",
            "my-fairlead.toml",
        ] {
            assert!(!is_config(path), "{path}");
        }
    }

    #[test]
    fn workflows_are_yaml_files_at_the_folder_top() {
        assert!(is_workflow(".github/workflows/ci.yml"));
        assert!(is_workflow(".github/workflows/nightly.yaml"));
        assert!(!is_workflow(".github/workflows/README.md"));
        assert!(!is_workflow(".github/workflows/sub/x.yml"));
        assert!(!is_workflow(".github/actions/x/action.yml"));
    }

    #[test]
    fn a_rename_reads_its_new_path_at_the_head_and_its_old_one_at_the_base() {
        let change = Change {
            path: "b.yml".into(),
            status: Status::Renamed,
            from: Some("a.yml".into()),
        };
        assert_eq!(
            sides(&change),
            [("b.yml", true, false), ("a.yml", false, true)]
        );
    }
}
