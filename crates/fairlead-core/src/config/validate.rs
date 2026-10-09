//! Rules the types can't express: unique ids, placeholders that exist, and
//! references between sections.

use std::collections::BTreeSet;

use super::Config;

const EXTRACTORS: [&str; 10] = [
    "vitest", "jest", "bun", "phpunit", "pest", "go", "pytest", "maven", "gradle", "regex",
];
const CWD_PLACEHOLDERS: [&str; 2] = ["module", "module.id"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub key: String,
    pub message: String,
}

pub(super) fn problem(key: impl Into<String>, message: impl Into<String>) -> Problem {
    Problem {
        key: key.into(),
        message: message.into(),
    }
}

/// The `{name}` captures in a glob, by the same rule `Pattern` matches with;
/// `{foo-bar}` is an alternation of one, not a capture.
fn placeholders(pattern: &str) -> BTreeSet<String> {
    let mut found = braces(pattern);
    found.retain(|inner| crate::pattern::is_capture(inner));
    found
}

/// Every `{...}` without a comma, for templates such as `cwd`, which aren't
/// globs and name `{module.id}`.
fn braces(pattern: &str) -> BTreeSet<String> {
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
    quarantine(config, &mut problems);
    replay(config, &mut problems);
    graph_edges(config, &mut problems);
    graph_providers(config, &mut problems);
    guard(config, &mut problems);
    memory(config, &mut problems);
    skills(config, &mut problems);
    super::stages::validate(config, &mut problems);
    agents(config, &mut problems);
    problems
}

/// The globs `fairlead plan` compiles, which would otherwise fail only once
/// a change reached them. Kept out of `validate`, which every load runs,
/// hooks included, since compiling them all costs more than a hook's budget.
pub fn plan_globs(config: &Config) -> Vec<Problem> {
    let mut problems = Vec::new();
    let problems = &mut problems;
    let tests = &config.tests;
    globs("tests.match", tests.matches.items(), problems);
    globs("tests.exclude", tests.exclude.items(), problems);
    for (i, runner) in tests.runners.items().iter().enumerate() {
        globs(
            &format!("tests.runners[{i}].match"),
            &runner.matches,
            problems,
        );
        globs(
            &format!("tests.runners[{i}].exclude"),
            &runner.exclude,
            problems,
        );
    }
    for (i, owner) in tests.owners.items().iter().enumerate() {
        if let Err(e) = crate::pattern::Pattern::new(&owner.matches) {
            problems.push(problem(format!("tests.owners[{i}].match"), e));
        }
        globs(
            &format!("tests.owners[{i}].covers"),
            &owner.covers,
            problems,
        );
    }
    globs("plan.run_all", config.plan.run_all.items(), problems);
    globs("plan.ignore", config.plan.ignore.items(), problems);
    std::mem::take(problems)
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
    if guard.budget_ms == 0 {
        problems.push(problem("guard.budget_ms", "must be at least 1"));
    }
    for (i, e) in guard.external.items().iter().enumerate() {
        if e.stages.contains(&super::Stage::Write) {
            problems.push(problem(
                format!("guard.external[{i}].stages"),
                "`write` isn't a stage an external rule can run at: the file isn't written yet",
            ));
        }
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
        if let Some(unless) = &c.unless {
            regexes(
                &format!("guard.commands[{i}].unless"),
                std::slice::from_ref(unless),
                problems,
            );
        }
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
fn graph_providers(config: &Config, problems: &mut Vec<Problem>) {
    let providers = config.graph.providers.items();
    for id in duplicates(providers.iter().map(|p| p.id.as_str())) {
        problems.push(problem(
            "graph.providers",
            format!("provider id `{id}` is used more than once"),
        ));
    }
    for (i, p) in providers.iter().enumerate() {
        let key = format!("graph.providers[{i}]");
        if p.id == "typescript" {
            problems.push(problem(
                format!("{key}.id"),
                "`typescript` is the built-in scanner's id",
            ));
        }
        if p.command.is_empty() {
            problems.push(problem(format!("{key}.command"), "is empty"));
        }
        if p.files.is_empty() {
            problems.push(problem(
                format!("{key}.files"),
                "is empty, so it claims nothing",
            ));
        }
        for (j, glob) in p.files.iter().enumerate() {
            if let Err(e) = crate::pattern::Pattern::new(glob) {
                problems.push(problem(format!("{key}.files[{j}]"), e));
            }
        }
    }
}

/// A `find` rule: its regex compiles with at most one capture, `{1}` is used
/// only with one, and its other `{name}`s are whole segments of `from`,
/// since the binding comes from the file the text was found in.
fn found_rule(
    rule: &crate::config::EdgeRule,
    find: &str,
    key: &str,
    names: &std::collections::BTreeSet<String>,
    problems: &mut Vec<Problem>,
) {
    let groups = match regex::Regex::new(find) {
        Ok(re) => re.captures_len() - 1,
        Err(e) => {
            problems.push(problem(format!("{key}.find"), e.to_string()));
            return;
        }
    };
    if groups > 1 {
        problems.push(problem(
            format!("{key}.find"),
            "may have at most one capture group; write the others as `(?:...)`",
        ));
    }
    let found = [crate::config::FOUND, crate::config::FOUND_PATH];
    for (j, to) in rule.targets().iter().enumerate() {
        let here = placeholders(to);
        if groups == 0 && here.iter().any(|n| found.contains(&n.as_str())) {
            problems.push(problem(
                format!("{key}.to[{j}]"),
                "uses `{1}`, but `find` captures nothing",
            ));
        }
        if here
            .iter()
            .any(|n| !found.contains(&n.as_str()) && !names.contains(n))
        {
            problems.push(problem(
                format!("{key}.to[{j}]"),
                "may use only `{1}`, `{1|path}` and the `{name}`s of `from`",
            ));
        }
    }
    if !names
        .iter()
        .all(|n| rule.from.split('/').any(|s| s == format!("{{{n}}}")))
    {
        problems.push(problem(
            key.to_string(),
            "with `find`, each `{name}` must be a whole path segment in `from`",
        ));
    }
    for glob in std::iter::once(&rule.from).chain(&rule.targets()) {
        if let Err(e) = crate::pattern::Pattern::new(glob) {
            problems.push(problem(key.to_string(), e));
        }
    }
}

fn graph_edges(config: &Config, problems: &mut Vec<Problem>) {
    let whole = |glob: &str, name: &str| glob.split('/').any(|s| s == format!("{{{name}}}"));
    for (i, rule) in config.graph.edges.items().iter().enumerate() {
        let key = format!("graph.edges[{i}]");
        let names = placeholders(&rule.from);
        if rule.to.is_empty() {
            problems.push(problem(format!("{key}.to"), "needs at least one glob"));
        }
        if let Some(find) = &rule.find {
            found_rule(rule, find, &key, &names, problems);
            continue;
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

/// Whether a `fairlead` pin names a later version than this binary, or
/// `None` when it isn't a version.
pub(crate) fn needs_later(pin: &str) -> Option<bool> {
    let parse = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    let want = parse(pin)?;
    let have = parse(env!("CARGO_PKG_VERSION")).expect("the crate version is a version");
    Some(want > have[..want.len().min(have.len())].to_vec())
}

pub(crate) fn needs_later_message(pin: &str) -> String {
    format!(
        "this config needs Fairlead {pin} or later; this is {}",
        env!("CARGO_PKG_VERSION")
    )
}

fn version_pin(config: &Config, problems: &mut Vec<Problem>) {
    let Some(pin) = &config.fairlead else { return };
    match needs_later(pin) {
        Some(true) => problems.push(problem("fairlead", needs_later_message(pin))),
        None => problems.push(problem(
            "fairlead",
            format!("`{pin}` isn't a version like \"0.2\""),
        )),
        Some(false) => {}
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
        runner_extras(i, runner, problems);
        if let Some(cwd) = &runner.cwd {
            for name in braces(cwd) {
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

/// `all_command` runs with no files to name, and `exclude_arg` exists to
/// name one, so each is checked for the placeholder it can use.
fn runner_extras(i: usize, runner: &super::Runner, problems: &mut Vec<Problem>) {
    if let Some(all) = &runner.all_command {
        let key = format!("tests.runners[{i}].all_command");
        if all.is_empty() {
            problems.push(problem(key.clone(), "is empty"));
        }
        for name in ["files", "class", "classes"] {
            if all.iter().any(|a| a.contains(&format!("{{{name}}}"))) {
                problems.push(problem(
                    key.clone(),
                    format!(
                        "`{{{name}}}` has nothing to expand to when everything runs; leave it out"
                    ),
                ));
            }
        }
    }
    if let Some(exclude) = &runner.exclude_arg {
        if !exclude.iter().any(|a| a.contains("{file}")) {
            problems.push(problem(
                format!("tests.runners[{i}].exclude_arg"),
                "needs `{file}`, where each held test file goes, such as [\"--exclude\", \"{file}\"]",
            ));
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
        let always = config.done.always.items().contains(&check.id);
        if check.paths.is_empty() && check.modules.is_empty() && !always {
            problems.push(problem(
                format!("checks[{i}]"),
                "needs `paths` or `modules`, or it never runs",
            ));
        }
    }
}

fn memory(config: &Config, problems: &mut Vec<Problem>) {
    let m = &config.memory;
    let dir = std::path::Path::new(&m.dir);
    if m.dir.trim().is_empty() || dir.is_absolute() || m.dir.split('/').any(|p| p == "..") {
        problems.push(problem(
            "memory.dir",
            "must be a path inside the repository",
        ));
    }
    for (key, value) in [("memory.max_lines", m.max_lines), ("memory.cap", m.cap)] {
        if value == 0 {
            problems.push(problem(key, "must be at least 1"));
        }
    }
    if m.review_days == 0 {
        problems.push(problem("memory.review_days", "must be at least 1"));
    }
}

/// The agents `skills sync` can write for.
pub const SKILL_TARGETS: [&str; 3] = ["claude", "agents", "cursor"];

fn skills(config: &Config, problems: &mut Vec<Problem>) {
    let s = &config.skills;
    if s.cap == 0 {
        problems.push(problem("skills.cap", "must be at least 1"));
    }
    if s.imports > 3 || s.importers > 3 {
        problems.push(problem(
            "skills.imports",
            "hops past 3 offer nearly everything; 0 to 3",
        ));
    }
    for (i, t) in s.targets.items().iter().enumerate() {
        if !SKILL_TARGETS.contains(&t.as_str()) {
            problems.push(problem(
                format!("skills.targets[{i}]"),
                format!("`{t}` isn't one of {}", SKILL_TARGETS.join(", ")),
            ));
        }
    }
    for (i, r) in s.routes.items().iter().enumerate() {
        let key = format!("skills.routes[{i}]");
        if !r.skill.ends_with("SKILL.md") || r.skill.starts_with('/') || r.skill.contains("..") {
            problems.push(problem(
                format!("{key}.skill"),
                "must be a SKILL.md path inside the repository",
            ));
        }
        if r.paths.is_empty() && r.modules.is_empty() && !r.always {
            problems.push(problem(
                key.clone(),
                "needs a scope: `paths`, `modules` or `always = true`",
            ));
        }
        globs(&format!("{key}.paths"), &r.paths, problems);
    }
    let skills: Vec<&str> = s.routes.items().iter().map(|r| r.skill.as_str()).collect();
    for dup in duplicates(skills.into_iter()) {
        problems.push(problem(
            "skills.routes",
            format!("`{dup}` is routed twice; give one route every scope"),
        ));
    }
}

/// A file the block goes in: relative, inside the repository, and a file.
fn agents(config: &Config, problems: &mut Vec<Problem>) {
    for (i, file) in config.agents.files.items().iter().enumerate() {
        let path = std::path::Path::new(file);
        let outside = path.is_absolute()
            || file.starts_with(['/', '\\'])
            || file.get(1..2) == Some(":")
            || file.split(['/', '\\']).any(|p| p == "..");
        if file.trim().is_empty() || outside || file.ends_with(['/', '\\']) {
            problems.push(problem(
                format!("agents.files[{i}]"),
                "must be a file path inside the repository",
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
        if !is_date(&entry.until) {
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
    for (i, id) in config.done.always.items().iter().enumerate() {
        if !check_ids.contains(id.as_str()) {
            problems.push(problem(
                format!("done.always[{i}]"),
                format!("no check has id `{id}`"),
            ));
        }
    }
}

/// `YYYY-MM-DD`, the form every `until` takes.
pub fn is_date(text: &str) -> bool {
    text.len() == 10
        && text.chars().enumerate().all(|(j, c)| {
            if j == 4 || j == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
}

fn quarantine(config: &Config, problems: &mut Vec<Problem>) {
    let check_ids: BTreeSet<&str> = config
        .checks
        .items()
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    for (i, entry) in config.quarantine.items().iter().enumerate() {
        let key = format!("quarantine[{i}]");
        match (&entry.path, &entry.check) {
            (Some(_), Some(_)) | (None, None) => problems.push(problem(
                key.clone(),
                "names one test file (`path`) or one check (`check`)",
            )),
            (Some(path), None) if path.trim().is_empty() || path.contains('*') => {
                problems.push(problem(
                    format!("{key}.path"),
                    "must name one test file, not a pattern",
                ))
            }
            (None, Some(id)) if !check_ids.contains(id.as_str()) => problems.push(problem(
                format!("{key}.check"),
                format!("no check has id `{id}`"),
            )),
            _ => {}
        }
        if entry.os.is_none() && entry.when.is_empty() {
            problems.push(problem(
                key.clone(),
                "names no `os` and no `when`, so it would hold everywhere",
            ));
        }
        regexes(
            &format!("{key}.signature"),
            std::slice::from_ref(&entry.signature),
            problems,
        );
        for (field, text, why) in [
            ("reason", &entry.reason, "must give the evidence"),
            (
                "proved_in",
                &entry.proved_in,
                "must say where it is proved instead",
            ),
        ] {
            if text.trim().is_empty() {
                problems.push(problem(format!("{key}.{field}"), why));
            }
        }
        if !is_date(&entry.until) {
            problems.push(problem(
                format!("{key}.until"),
                "must be a date, YYYY-MM-DD",
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
    fn placeholders_are_only_the_names_a_pattern_captures() {
        let glob = "test/{foo-bar}/{area}/**";
        let captured = crate::pattern::Pattern::new(glob)
            .unwrap()
            .captures_names()
            .unwrap();
        assert_eq!(placeholders(glob), captured);
        assert_eq!(
            placeholders(glob).into_iter().collect::<Vec<_>>(),
            vec!["area".to_string()]
        );
    }

    fn runner_problems(extra: &str) -> Vec<Problem> {
        let text = format!(
            "[[tests.runners]]\nid = \"unit\"\nmatch = [\"t/*.test.ts\"]\ncommand = [\"run\", \"{{files}}\"]\n{extra}"
        );
        let config: Config = toml::from_str(&text).expect("the config parses");
        validate(&config)
            .into_iter()
            .filter(|p| p.key.starts_with("tests.runners"))
            .collect()
    }

    #[test]
    fn an_all_command_naming_files_or_classes_is_a_problem() {
        let problems = runner_problems("all_command = [\"run\", \"{files}\"]\n");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].key, "tests.runners[0].all_command");
        assert!(problems[0]
            .message
            .contains("`{files}` has nothing to expand to"));
        assert_eq!(
            runner_problems("all_command = [\"-Dtest={classes}\"]\n").len(),
            1
        );
        let fine = "all_command = [\"run\", \"--dir\", \"{module}\", \"{packages}\"]\n";
        assert_eq!(runner_problems(fine), Vec::new());
    }

    #[test]
    fn an_exclude_arg_without_a_file_placeholder_is_a_problem() {
        let problems = runner_problems("exclude_arg = [\"--exclude\"]\n");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].key, "tests.runners[0].exclude_arg");
        assert_eq!(
            runner_problems("exclude_arg = [\"--ignore={file}\"]\n"),
            Vec::new()
        );
    }

    #[test]
    fn agents_files_must_stay_inside_the_repository_and_write_takes_two_values() {
        let keys = |files: &str| -> Vec<String> {
            let config: Config =
                toml::from_str(&format!("[agents]\nfiles = {files}\n")).expect("parses");
            validate(&config).into_iter().map(|p| p.key).collect()
        };
        assert!(keys(r#"["AGENTS.md", "docs/AGENTS.md"]"#).is_empty());
        assert_eq!(
            keys(r#"["/etc/AGENTS.md", "../AGENTS.md", "", "docs/", "C:\\x.md"]"#),
            (0..5)
                .map(|i| format!("agents.files[{i}]"))
                .collect::<Vec<_>>()
        );
        assert!(toml::from_str::<Config>("[agents]\nwrite = \"never\"\n").is_ok());
        assert!(toml::from_str::<Config>("[agents]\nwrite = \"sometimes\"\n").is_err());
    }

    #[test]
    fn duplicates_are_reported_once_each() {
        assert_eq!(
            duplicates(["a", "b", "a", "a"].into_iter()),
            vec!["a".to_string()]
        );
    }
}
