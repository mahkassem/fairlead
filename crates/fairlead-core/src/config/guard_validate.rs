//! Validation of `[guard]`: globs and regexes that compile, rules that
//! turn something on, and rule ids that are unique.

use super::validate::{duplicates, globs, problem, Problem};
use super::{Config, Guard};

pub(super) fn guard(config: &Config, problems: &mut Vec<Problem>) {
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
        if e.stages().contains(&super::Stage::Write) {
            problems.push(problem(
                format!("guard.external[{i}].stages"),
                "`write` isn't a stage an external rule can run at: the file isn't written yet",
            ));
        }
    }
    if let super::PathsSetting::Rules(p) = &guard.paths {
        globs("guard.paths.deny", p.deny.items(), problems);
        globs("guard.paths.allow", p.allow.items(), problems);
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
    let ids = external_rules(guard, problems);
    let external_ids: Vec<&str> = ids.iter().map(String::as_str).collect();
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
pub const GUARD_RULES: [&str; 14] = [
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
    "secret-path",
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

pub(super) fn regexes(key: &str, patterns: &[String], problems: &mut Vec<Problem>) {
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

/// Each `[[guard.external]]` rule: a new id, one of a command or a preset,
/// a stage to run at, and `diff` only on a command.
fn external_rules(guard: &Guard, problems: &mut Vec<Problem>) -> Vec<String> {
    let externals = guard.external.items();
    let ids: Vec<String> = externals.iter().map(|e| e.rule_id()).collect();
    for id in duplicates(ids.iter().map(String::as_str)) {
        problems.push(problem(
            "guard.external",
            format!("rule id `{id}` is used more than once"),
        ));
    }
    for (i, e) in externals.iter().enumerate() {
        let key = format!("guard.external[{i}]");
        let id = e.rule_id();
        if GUARD_RULES.contains(&id.as_str()) || id.trim().is_empty() {
            problems.push(problem(format!("{key}.id"), "must be a new rule id"));
        }
        match (e.preset, e.command.is_empty()) {
            (None, true) => problems.push(problem(
                format!("{key}.command"),
                "is empty; give a command, or a `preset` such as \"gitleaks\"",
            )),
            (Some(p), false) => problems.push(problem(
                format!("{key}.command"),
                format!(
                    "is set beside `preset = \"{}\"`, which runs the tool itself; keep one",
                    p.name()
                ),
            )),
            _ => {}
        }
        if e.stages().is_empty() {
            problems.push(problem(
                format!("{key}.stages"),
                "names no stage, so it never runs",
            ));
        }
        if e.diff && (e.preset.is_some() || e.ratchet) {
            problems.push(problem(
                format!("{key}.diff"),
                "compares a command's findings with the base's; a preset or a ratchet already keeps old findings out",
            ));
        }
    }
    ids
}
