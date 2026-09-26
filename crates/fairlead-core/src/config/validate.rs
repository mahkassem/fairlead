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
        if size.file_lines.is_none() && size.function_lines.is_none() {
            problems.push(problem(
                "guard.size",
                "sets no limit, such as `file_lines` or `function_lines`",
            ));
        }
        for (key, limit) in [
            ("file_lines", size.file_lines),
            ("function_lines", size.function_lines),
        ] {
            if limit == Some(0) {
                problems.push(problem(format!("guard.size.{key}"), "must be at least 1"));
            }
        }
    }
    if let Some(comments) = &guard.comments {
        comment_rules(comments, problems);
    }
    if let Some(t) = &guard.test_names {
        test_names(t, problems);
    }
    if let Some(c) = &guard.citations {
        citations(c, problems);
    }
    if let Some(m) = &guard.migrations {
        migrations(m, problems);
    }
    for (i, c) in guard.commands.items().iter().enumerate() {
        regexes(
            &format!("guard.commands[{i}].match"),
            std::slice::from_ref(&c.matches),
            problems,
        );
        if c.reason.trim().is_empty() {
            problems.push(problem(
                format!("guard.commands[{i}].reason"),
                "must say why",
            ));
        }
    }
    let externals = guard.external.items();
    for id in duplicates(externals.iter().map(|e| e.id.as_str())) {
        problems.push(problem(
            "guard.external",
            format!("rule id `{id}` is used more than once"),
        ));
    }
    for (i, e) in externals.iter().enumerate() {
        let key = format!("guard.external[{i}]");
        if GUARD_RULES.contains(&e.id.as_str()) || e.id.trim().is_empty() {
            problems.push(problem(format!("{key}.id"), "must be a new rule id"));
        }
        if e.command.is_empty() {
            problems.push(problem(format!("{key}.command"), "is empty"));
        }
        if e.stages.is_empty() {
            problems.push(problem(
                format!("{key}.stages"),
                "names no stage, so it never runs",
            ));
        }
    }
    let external_ids: Vec<&str> = externals.iter().map(|e| e.id.as_str()).collect();
    for rule in guard.cite.keys() {
        if !GUARD_RULES.contains(&rule.as_str()) && !external_ids.contains(&rule.as_str()) {
            problems.push(problem(
                format!("guard.cite.{rule}"),
                format!("isn't a rule; the rules are {}", GUARD_RULES.join(", ")),
            ));
        }
    }
}

/// Every rule id a guard preset can report.
pub const GUARD_RULES: [&str; 13] = [
    "file-length",
    "function-length",
    "block-length",
    "density",
    "history",
    "item-code",
    "agent-instruction",
    "block-marker",
    "test-file-name",
    "test-title",
    "citation",
    "migration-edit",
    "migration-prefix",
];

fn test_names(t: &super::TestNames, problems: &mut Vec<Problem>) {
    const KEY: &str = "guard.test_names";
    if t.files.items().is_empty() {
        problems.push(problem(format!("{KEY}.files"), "needs at least one glob"));
    }
    globs(&format!("{KEY}.files"), t.files.items(), problems);
    globs(&format!("{KEY}.exclude"), t.exclude.items(), problems);
    if t.file.is_none() && t.titles_without.is_none() {
        problems.push(problem(KEY, "sets neither `file` nor `titles_without`"));
    }
    for (name, pattern) in [("file", &t.file), ("titles_without", &t.titles_without)] {
        if let Some(p) = pattern {
            regexes(&format!("{KEY}.{name}"), std::slice::from_ref(p), problems);
        }
    }
}

fn citations(c: &super::Citations, problems: &mut Vec<Problem>) {
    const KEY: &str = "guard.citations";
    if c.files.items().is_empty() {
        problems.push(problem(format!("{KEY}.files"), "needs at least one glob"));
    }
    globs(&format!("{KEY}.files"), c.files.items(), problems);
    globs(&format!("{KEY}.exclude"), c.exclude.items(), problems);
    match regex::Regex::new(&c.pattern) {
        Ok(re) if re.capture_names().flatten().any(|n| n == "code") => {}
        Ok(_) => problems.push(problem(
            format!("{KEY}.pattern"),
            "needs a group named `code`",
        )),
        Err(e) => problems.push(problem(format!("{KEY}.pattern"), e.to_string())),
    }
    if c.headings_in.trim().is_empty() {
        problems.push(problem(
            format!("{KEY}.headings_in"),
            "must name a Markdown file",
        ));
    }
}

fn migrations(m: &super::Migrations, problems: &mut Vec<Problem>) {
    const KEY: &str = "guard.migrations";
    if m.files.items().is_empty() {
        problems.push(problem(format!("{KEY}.files"), "needs at least one glob"));
    }
    globs(&format!("{KEY}.files"), m.files.items(), problems);
    if !m.immutable && m.unique_prefix.is_none() {
        problems.push(problem(KEY, "turns on no rule"));
    }
    if let Some(u) = &m.unique_prefix {
        if u.allow.iter().any(|group| group.len() < 2) {
            problems.push(problem(
                format!("{KEY}.unique_prefix.allow"),
                "each group names two files or more",
            ));
        }
    }
}

fn regexes(key: &str, patterns: &[String], problems: &mut Vec<Problem>) {
    for (i, pattern) in patterns.iter().enumerate() {
        match regex::Regex::new(pattern) {
            Err(e) => problems.push(problem(format!("{key}[{i}]"), e.to_string())),
            Ok(re) if re.is_match("") => {
                problems.push(problem(format!("{key}[{i}]"), "matches an empty string"))
            }
            Ok(_) => {}
        }
    }
}

fn comment_rules(c: &super::CommentRules, problems: &mut Vec<Problem>) {
    const KEY: &str = "guard.comments";
    if c.files.items().is_empty() {
        problems.push(problem(format!("{KEY}.files"), "needs at least one glob"));
    }
    for (name, list) in [
        ("files", &c.files),
        ("exclude", &c.exclude),
        ("tests", &c.tests),
        ("migrations", &c.migrations),
    ] {
        globs(&format!("{KEY}.{name}"), list.items(), problems);
    }
    let any = c.block_length.is_some()
        || c.density.is_some()
        || c.history.is_some()
        || c.item_codes.is_some()
        || !c.agent_phrases.items().is_empty()
        || c.block_marker;
    if !any {
        problems.push(problem(KEY, "turns on no rule"));
    }
    if let Some(b) = &c.block_length {
        let limits = [Some(b.source), b.test, b.header, b.inline, b.migration];
        if limits.contains(&Some(0)) {
            problems.push(problem(
                format!("{KEY}.block_length"),
                "every limit must be at least 1",
            ));
        }
    }
    if let Some(d) = &c.density {
        let share = |v: f64| v > 0.0 && v <= 1.0;
        if !share(d.source) || d.test.is_some_and(|t| !share(t)) {
            problems.push(problem(
                format!("{KEY}.density"),
                "a share is above 0 and at most 1",
            ));
        }
    }
    if let Some(h) = &c.history {
        if !h.dates && !h.measured && h.names.is_empty() && h.phrases.is_empty() {
            problems.push(problem(format!("{KEY}.history"), "checks nothing"));
        }
        if h.names
            .iter()
            .chain(&h.phrases)
            .any(|w| w.trim().is_empty())
        {
            problems.push(problem(
                format!("{KEY}.history"),
                "a name or phrase is empty",
            ));
        }
    }
    if let Some(codes) = &c.item_codes {
        regexes(
            &format!("{KEY}.item_codes.pattern"),
            std::slice::from_ref(&codes.pattern),
            problems,
        );
    }
    regexes(
        &format!("{KEY}.agent_phrases"),
        c.agent_phrases.items(),
        problems,
    );
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
