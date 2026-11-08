//! The `[pr]` rules, one engine for three places: `fairlead done`, the
//! pre-push hook, and `fairlead pr check` in CI. Each place gathers the
//! facts it can know; a rule whose facts are missing is reported as not
//! checked, never as passed.

use fairlead_core::config::{Place, Pr};
use fairlead_core::pattern::Pattern;
use regex::Regex;

/// What a place knows about a change. `None` is "can't tell here".
#[derive(Debug, Default, Clone)]
pub struct Facts {
    pub branch: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub labels: Option<Vec<String>>,
    /// The branch the pull request targets.
    pub base: Option<String>,
    /// Whether the branch contains the base's head.
    pub contains_base: Option<bool>,
    /// Paths the change adds or changes.
    pub changed: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub rule: &'static str,
    pub what: String,
    pub fix: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub failures: Vec<Failure>,
    /// Rules this place can't check, and why.
    pub unchecked: Vec<String>,
    /// Rules that held.
    pub passed: usize,
}

impl Report {
    fn fail(&mut self, rule: &'static str, what: String, fix: String) {
        self.failures.push(Failure { rule, what, fix });
    }

    fn skip(&mut self, what: &str) {
        if !self.unchecked.iter().any(|u| u == what) {
            self.unchecked.push(what.to_string());
        }
    }

    /// The report as lines: each failure with its fix, then what wasn't checked.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for f in &self.failures {
            out.push_str(&format!("  {}: {}\n    fix: {}\n", f.rule, f.what, f.fix));
        }
        if !self.unchecked.is_empty() {
            out.push_str(&format!(
                "  not checked here: {}\n",
                self.unchecked.join("; ")
            ));
        }
        out
    }
}

fn regex(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

/// A Markdown heading line's text, without its `#`s or a closing run of them.
fn heading(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix('#')?;
    let rest = rest.trim_start_matches('#');
    rest.starts_with(' ')
        .then(|| rest.trim().trim_end_matches('#').trim().to_lowercase())
}

pub fn check(pr: &Pr, facts: &Facts) -> Report {
    let mut r = Report::default();
    names(pr, facts, &mut r);
    ticket(pr, facts, &mut r);
    if !pr.body_headings.is_empty() {
        match &facts.body {
            None => r.skip("body headings (no pull request body)"),
            Some(body) => {
                let have: Vec<String> = body.lines().filter_map(heading).collect();
                let missing: Vec<&String> = pr
                    .body_headings
                    .iter()
                    .filter(|h| {
                        !have.contains(&h.trim().trim_start_matches('#').trim().to_lowercase())
                    })
                    .collect();
                if missing.is_empty() {
                    r.passed += 1;
                } else {
                    let list: Vec<String> = missing.iter().map(|h| format!("`## {h}`")).collect();
                    r.fail(
                        "pr-body",
                        format!("the body has no heading {}", list.join(", ")),
                        format!("add {} to the pull request description", list.join(" and ")),
                    );
                }
            }
        }
    }
    base(pr, facts, &mut r);
    labels(pr, facts, &mut r);
    files(pr, facts, &mut r);
    r
}

fn names(pr: &Pr, facts: &Facts, r: &mut Report) {
    for (rule, place, pattern, value) in [
        ("pr-branch", "branch name", &pr.branch, &facts.branch),
        ("pr-title", "title", &pr.title, &facts.title),
    ] {
        let Some(re) = pattern.as_deref().and_then(regex) else {
            continue;
        };
        match value {
            None => r.skip(&format!("{place} (not known here)")),
            Some(v) if re.is_match(v) => r.passed += 1,
            Some(v) => r.fail(
                rule,
                format!("the {place} `{v}` doesn't match `{}`", re.as_str()),
                if rule == "pr-branch" {
                    "rename the branch to match: `git branch -m <new-name>`, then push it under the new name".to_string()
                } else {
                    "edit the pull request title to match".to_string()
                },
            ),
        }
    }
}

fn ticket(pr: &Pr, facts: &Facts, r: &mut Report) {
    let Some(t) = &pr.ticket else { return };
    let Some(re) = regex(&t.pattern) else { return };
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for place in &t.places {
        let value = match place {
            Place::Branch => &facts.branch,
            Place::Title => &facts.title,
            Place::Body => &facts.body,
        };
        match value {
            None => r.skip(&format!(
                "ticket key in the {} (not known here)",
                place.name()
            )),
            Some(v) if re.is_match(v) => found.push(place.name()),
            Some(_) => missing.push(place.name()),
        }
    }
    let ok = if t.any {
        !found.is_empty() || missing.is_empty()
    } else {
        missing.is_empty()
    };
    if ok {
        if !found.is_empty() {
            r.passed += 1;
        }
        return;
    }
    let want = if t.any { " or " } else { " and " };
    let places: Vec<&str> = t.places.iter().map(|p| p.name()).collect();
    r.fail(
        "pr-ticket",
        format!(
            "no ticket key matching `{}` in the {}",
            t.pattern,
            missing.join(want)
        ),
        format!("put the ticket key in the {}", places.join(want)),
    );
}

fn base(pr: &Pr, facts: &Facts, r: &mut Report) {
    let Some(want) = &pr.base else { return };
    if let Some(got) = &facts.base {
        if got == want {
            r.passed += 1;
        } else {
            r.fail(
                "pr-base",
                format!("the pull request targets `{got}`, not `{want}`"),
                format!("change the pull request's base to `{want}`"),
            );
        }
    }
    if !pr.up_to_date {
        return;
    }
    match facts.contains_base {
        None => r.skip(&format!("whether the branch contains `{want}`'s head (not fetched)")),
        Some(true) => r.passed += 1,
        Some(false) => r.fail(
            "pr-stale",
            format!("the branch doesn't contain `{want}`'s head: it's behind it, or was cut from another branch"),
            format!("`git fetch origin {want}` and rebase onto, or merge, `origin/{want}`"),
        ),
    }
}

fn labels(pr: &Pr, facts: &Facts, r: &mut Report) {
    let l = &pr.labels;
    if l.require.is_empty() && l.require_any.is_empty() && l.forbid.is_empty() {
        return;
    }
    let Some(have) = &facts.labels else {
        r.skip("labels (no pull request)");
        return;
    };
    let has = |x: &String| have.contains(x);
    let missing: Vec<&String> = l.require.iter().filter(|x| !has(x)).collect();
    let forbidden: Vec<&String> = l.forbid.iter().filter(|x| has(x)).collect();
    let none_of = !l.require_any.is_empty() && !l.require_any.iter().any(has);
    for x in &missing {
        r.fail(
            "pr-labels",
            format!("the label `{x}` is missing"),
            format!("add the label `{x}`"),
        );
    }
    if none_of {
        r.fail(
            "pr-labels",
            format!("none of the labels {} is set", l.require_any.join(", ")),
            "add one of them".to_string(),
        );
    }
    for x in &forbidden {
        r.fail(
            "pr-labels",
            format!("the label `{x}` is set"),
            format!("remove the label `{x}` when the change is ready"),
        );
    }
    if missing.is_empty() && forbidden.is_empty() && !none_of {
        r.passed += 1;
    }
}

fn compile(globs: &[String]) -> Vec<Pattern> {
    globs.iter().filter_map(|g| Pattern::new(g).ok()).collect()
}

fn files(pr: &Pr, facts: &Facts, r: &mut Report) {
    for f in &pr.files {
        let paths = compile(&f.path);
        let when = compile(&f.when);
        let shown = f.path.join(", ");
        let Some(changed) = &facts.changed else {
            r.skip(&format!(
                "a file matching {shown} (the changed files aren't known)"
            ));
            continue;
        };
        let applies = when.is_empty() || changed.iter().any(|c| when.iter().any(|p| p.is_match(c)));
        if !applies || changed.iter().any(|c| paths.iter().any(|p| p.is_match(c))) {
            r.passed += 1;
            continue;
        }
        let excused = f
            .unless_label
            .as_ref()
            .map(|label| facts.labels.as_ref().map(|have| have.contains(label)));
        match excused {
            Some(Some(true)) => {
                r.passed += 1;
                continue;
            }
            Some(None) => {
                let label = f.unless_label.as_deref().unwrap_or_default();
                r.skip(&format!(
                    "a file matching {shown}, unless labelled `{label}` (labels aren't known here, and none was added)"
                ));
                continue;
            }
            _ => {}
        }
        let why = f
            .why
            .as_deref()
            .map(|w| format!(" ({w})"))
            .unwrap_or_default();
        let or_label = f
            .unless_label
            .as_deref()
            .map(|l| format!(", or label the pull request `{l}`"))
            .unwrap_or_default();
        r.fail(
            "pr-file",
            format!("the change adds no file matching {shown}{why}"),
            format!("add one{or_label}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fairlead_core::config::{Labels, RequiredFile, Ticket};

    fn rules() -> Pr {
        Pr {
            base: Some("main".into()),
            branch: Some(r"^(feat|fix)/[a-z0-9-]+$".into()),
            title: Some(r"^(feat|fix): .+".into()),
            ticket: Some(Ticket {
                pattern: r"[A-Z]+-[0-9]+".into(),
                places: vec![Place::Title, Place::Body],
                any: true,
            }),
            body_headings: vec!["Summary".into(), "Testing".into()],
            labels: Labels {
                forbid: vec!["do-not-merge".into()],
                ..Labels::default()
            },
            files: vec![RequiredFile {
                path: vec!["changes/*.md".into()],
                when: vec!["src/**".into()],
                unless_label: Some("no-change-note".into()),
                why: None,
            }],
            ..Pr::default()
        }
    }

    fn all_known() -> Facts {
        Facts {
            branch: Some("feat/add-export".into()),
            title: Some("feat: add export (ABC-12)".into()),
            body: Some("## Summary\nadds it\n\n### testing ###\nran it\n".into()),
            labels: Some(vec![]),
            base: Some("main".into()),
            contains_base: Some(true),
            changed: Some(vec!["src/a.ts".into(), "changes/export.md".into()]),
        }
    }

    #[test]
    fn a_change_that_meets_every_rule_passes_them_all() {
        let report = check(&rules(), &all_known());
        assert!(report.failures.is_empty(), "{}", report.render());
        assert!(report.unchecked.is_empty());
        assert_eq!(report.passed, 8);
    }

    #[test]
    fn each_broken_rule_says_what_failed_and_how_to_fix_it() {
        let facts = Facts {
            branch: Some("Feature/X".into()),
            title: Some("Add export".into()),
            body: Some("## Summary\n".into()),
            labels: Some(vec!["do-not-merge".into()]),
            base: Some("develop".into()),
            contains_base: Some(false),
            changed: Some(vec!["src/a.ts".into()]),
        };
        let report = check(&rules(), &facts);
        let rules: Vec<&str> = report.failures.iter().map(|f| f.rule).collect();
        assert_eq!(
            rules,
            [
                "pr-branch",
                "pr-title",
                "pr-ticket",
                "pr-body",
                "pr-base",
                "pr-stale",
                "pr-labels",
                "pr-file"
            ]
        );
        let text = report.render();
        assert!(text.contains("`## Testing`"), "{text}");
        assert!(
            text.contains("or label the pull request `no-change-note`"),
            "{text}"
        );
    }

    #[test]
    fn locally_unknown_facts_are_not_checked_rather_than_passed() {
        let facts = Facts {
            branch: Some("feat/x".into()),
            contains_base: Some(true),
            changed: Some(vec!["src/a.ts".into()]),
            ..Facts::default()
        };
        let report = check(&rules(), &facts);
        assert!(report.failures.is_empty(), "{}", report.render());
        let text = report.unchecked.join("; ");
        for what in [
            "title",
            "body headings",
            "labels",
            "unless labelled `no-change-note`",
        ] {
            assert!(text.contains(what), "{what}: {text}");
        }
    }

    #[test]
    fn a_ticket_in_every_place_is_needed_unless_any_is_set() {
        let mut pr = Pr {
            ticket: Some(Ticket {
                pattern: r"[A-Z]+-[0-9]+".into(),
                places: vec![Place::Branch, Place::Title],
                any: false,
            }),
            ..Pr::default()
        };
        let facts = Facts {
            branch: Some("feat/ABC-1-x".into()),
            title: Some("feat: x".into()),
            ..Facts::default()
        };
        assert_eq!(check(&pr, &facts).failures.len(), 1);
        pr.ticket.as_mut().unwrap().any = true;
        assert!(check(&pr, &facts).failures.is_empty());
    }

    #[test]
    fn a_label_excuses_the_required_file() {
        let mut facts = all_known();
        facts.changed = Some(vec!["src/a.ts".into()]);
        facts.labels = Some(vec!["no-change-note".into()]);
        assert!(check(&rules(), &facts).failures.is_empty());
        facts.changed = Some(vec!["docs/a.md".into()]);
        facts.labels = Some(vec![]);
        assert!(
            check(&rules(), &facts).failures.is_empty(),
            "outside `when`"
        );
    }
}
