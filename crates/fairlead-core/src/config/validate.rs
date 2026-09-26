//! Rules the types can't express: unique ids, placeholders that exist, and
//! references between sections.

use std::collections::BTreeSet;

use super::Config;

const EXTRACTORS: [&str; 4] = ["vitest", "jest", "bun", "regex"];
const CWD_PLACEHOLDERS: [&str; 2] = ["module", "module.id"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub key: String,
    pub message: String,
}

fn problem(key: impl Into<String>, message: impl Into<String>) -> Problem {
    Problem {
        key: key.into(),
        message: message.into(),
    }
}

/// The `{name}` style placeholders in a pattern.
fn placeholders(pattern: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let inner = &after[..close];
        // Brace globs like `{ts,tsx}` aren't placeholders.
        if !inner.is_empty() && !inner.contains(',') {
            found.insert(inner.to_string());
        }
        rest = &after[close + 1..];
    }
    found
}

fn duplicates<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut dups = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            dups.insert(id.to_string());
        }
    }
    dups.into_iter().collect()
}

pub fn validate(config: &Config) -> Vec<Problem> {
    let mut problems = Vec::new();
    version_pin(config, &mut problems);
    for (i, def) in config.modules.define.items().iter().enumerate() {
        if !placeholders(&def.pattern).contains("name") {
            problems.push(problem(
                format!("modules.define[{i}].pattern"),
                "must contain `{name}`",
            ));
        }
    }
    runners(config, &mut problems);
    for (i, owner) in config.tests.owners.items().iter().enumerate() {
        let captured = placeholders(&owner.matches);
        for (j, cover) in owner.covers.iter().enumerate() {
            for name in placeholders(cover).difference(&captured) {
                problems.push(problem(
                    format!("tests.owners[{i}].covers[{j}]"),
                    format!("`{{{name}}}` isn't captured by `match`"),
                ));
            }
        }
    }
    checks(config, &mut problems);
    replay(config, &mut problems);
    graph_edges(config, &mut problems);
    guard(config, &mut problems);
    problems
}

fn globs(key: &str, globs: &[String], problems: &mut Vec<Problem>) {
    for (i, glob) in globs.iter().enumerate() {
        if let Err(e) = crate::pattern::Pattern::new(glob) {
            problems.push(problem(format!("{key}[{i}]"), e));
        }
    }
}

fn guard(config: &Config, problems: &mut Vec<Problem>) {
    let guard = &config.guard;
    if guard.baseline.trim().is_empty() {
        problems.push(problem("guard.baseline", "must name a file"));
    }
    globs("guard.exclude", guard.exclude.items(), problems);
    if let Some(size) = &guard.size {
        if size.files.items().is_empty() {
            problems.push(problem("guard.size.files", "needs at least one glob"));
        }
        globs("guard.size.files", size.files.items(), problems);
        globs("guard.size.exclude", size.exclude.items(), problems);
        match size.file_lines {
            None => problems.push(problem("guard.size", "sets no limit, such as `file_lines`")),
            Some(0) => problems.push(problem("guard.size.file_lines", "must be at least 1")),
            Some(_) => {}
        }
    }
}

/// A placeholder is read from a side where it is a whole path segment, since
/// `{area}*` would also swallow `-runs` from `area-runs`; every glob naming
/// one must name them all, or a leftover `{name}` would match anything.
fn graph_edges(config: &Config, problems: &mut Vec<Problem>) {
    let whole = |glob: &str, name: &str| glob.split('/').any(|s| s == format!("{{{name}}}"));
    for (i, rule) in config.graph.edges.items().iter().enumerate() {
        let key = format!("graph.edges[{i}]");
        let names = placeholders(&rule.from);
        if rule.to.is_empty() {
            problems.push(problem(format!("{key}.to"), "needs at least one glob"));
        }
        for (j, to) in rule.to.iter().enumerate() {
            let here = placeholders(to);
            if !here.is_empty() && here != names {
                problems.push(problem(
                    format!("{key}.to[{j}]"),
                    "must use the same `{name}`s as `from`, or none",
                ));
            }
        }
        let bound = rule.to.iter().filter(|t| !placeholders(t).is_empty());
        let from_side = names.iter().all(|n| whole(&rule.from, n));
        let to_side = names.iter().all(|n| bound.clone().all(|t| whole(t, n)))
            && bound.clone().next().is_some();
        if !names.is_empty() && !from_side && !to_side {
            problems.push(problem(
                key.clone(),
                "each `{name}` must be a whole path segment in `from`, or in every `to` that uses it",
            ));
        }
        for glob in std::iter::once(&rule.from).chain(&rule.to) {
            if let Err(e) = crate::pattern::Pattern::new(glob) {
                problems.push(problem(key.clone(), e));
            }
        }
    }
    for (i, glob) in config.graph.barrier.items().iter().enumerate() {
        if let Err(e) = crate::pattern::Pattern::new(glob) {
            problems.push(problem(format!("graph.barrier[{i}]"), e));
        }
    }
}

fn version_pin(config: &Config, problems: &mut Vec<Problem>) {
    let Some(pin) = &config.fairlead else { return };
    let parse = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    match (parse(pin), parse(env!("CARGO_PKG_VERSION"))) {
        (Some(want), Some(have)) if want > have[..want.len().min(have.len())].to_vec() => {
            problems.push(problem(
                "fairlead",
                format!(
                    "this config needs Fairlead {pin} or later; this is {}",
                    env!("CARGO_PKG_VERSION")
                ),
            ));
        }
        (None, _) => problems.push(problem(
            "fairlead",
            format!("`{pin}` isn't a version like \"0.2\""),
        )),
        _ => {}
    }
}

fn runners(config: &Config, problems: &mut Vec<Problem>) {
    let runners = config.tests.runners.items();
    for id in duplicates(runners.iter().map(|r| r.id.as_str())) {
        problems.push(problem(
            "tests.runners",
            format!("runner id `{id}` is used more than once"),
        ));
    }
    for (i, runner) in runners.iter().enumerate() {
        if runner.command.is_empty() {
            problems.push(problem(format!("tests.runners[{i}].command"), "is empty"));
        }
        if runner.matches.is_empty() {
            problems.push(problem(format!("tests.runners[{i}].match"), "is empty"));
        }
        if let Some(cwd) = &runner.cwd {
            for name in placeholders(cwd) {
                if !CWD_PLACEHOLDERS.contains(&name.as_str()) {
                    problems.push(problem(
                        format!("tests.runners[{i}].cwd"),
                        format!("unknown placeholder `{{{name}}}`"),
                    ));
                }
            }
        }
    }
}

fn checks(config: &Config, problems: &mut Vec<Problem>) {
    let checks = config.checks.items();
    for id in duplicates(checks.iter().map(|c| c.id.as_str())) {
        problems.push(problem(
            "checks",
            format!("check id `{id}` is used more than once"),
        ));
    }
    for (i, check) in checks.iter().enumerate() {
        if check.command.is_empty() {
            problems.push(problem(format!("checks[{i}].command"), "is empty"));
        }
        if check.paths.is_empty() && check.modules.is_empty() {
            problems.push(problem(
                format!("checks[{i}]"),
                "needs `paths` or `modules`, or it never runs",
            ));
        }
    }
}

fn replay(config: &Config, problems: &mut Vec<Problem>) {
    let runner_ids: BTreeSet<&str> = config
        .tests
        .runners
        .items()
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    for (i, source) in config.replay.failures.items().iter().enumerate() {
        let key = format!("replay.failures[{i}]");
        if !EXTRACTORS.contains(&source.extractor.as_str()) {
            problems.push(problem(
                format!("{key}.extractor"),
                format!(
                    "`{}` isn't one of {}",
                    source.extractor,
                    EXTRACTORS.join(", ")
                ),
            ));
        }
        if source.extractor == "regex" && source.pattern.is_none() {
            problems.push(problem(
                format!("{key}.pattern"),
                "is required with `extractor = \"regex\"`",
            ));
        }
        if !runner_ids.contains(source.runner.as_str()) {
            problems.push(problem(
                format!("{key}.runner"),
                format!("no runner has id `{}`", source.runner),
            ));
        }
    }
    for (i, entry) in config.replay.quarantine.items().iter().enumerate() {
        let key = format!("replay.quarantine[{i}]");
        let date = entry.until.len() == 10
            && entry.until.chars().enumerate().all(|(j, c)| {
                if j == 4 || j == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }
            });
        if !date {
            problems.push(problem(
                format!("{key}.until"),
                "must be a date, YYYY-MM-DD",
            ));
        }
        if entry.reason.trim().is_empty() {
            problems.push(problem(format!("{key}.reason"), "must give the evidence"));
        }
        if entry.path.trim().is_empty() || entry.path.contains('*') {
            problems.push(problem(
                format!("{key}.path"),
                "must name one test file, not a pattern",
            ));
        }
    }
    let check_ids: BTreeSet<&str> = config
        .checks
        .items()
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    for (i, step) in config.replay.checks.items().iter().enumerate() {
        if !check_ids.contains(step.check.as_str()) {
            problems.push(problem(
                format!("replay.checks[{i}].check"),
                format!("no check has id `{}`", step.check),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_skip_brace_globs() {
        let found = placeholders("services/{name}/test/**/*.{ts,tsx}");
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            vec!["name".to_string()]
        );
    }

    #[test]
    fn duplicates_are_reported_once_each() {
        assert_eq!(
            duplicates(["a", "b", "a", "a"].into_iter()),
            vec!["a".to_string()]
        );
    }
}
