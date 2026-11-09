//! `[[needs]]` in the done gate (#259): a planned test or check whose need
//! isn't met on this machine isn't run here. A need that names `proved_in`
//! lets the gate pass without them, since that place proves them before a
//! merge; one that doesn't keeps the gate open until it's met or says where.

use std::collections::BTreeMap;
use std::path::Path;

use fairlead_core::pattern::Pattern;
use fairlead_core::plan::{Invocation, InvocationKind};
use fairlead_guard::events::Unmet;

use crate::plan_cmd::Planned;

/// What one unmet need left out of one step.
#[derive(Debug, Clone, PartialEq)]
pub struct Skip {
    pub need: String,
    pub step: String,
    /// Test files left out; 0 for a check.
    pub files: usize,
    /// What the need's check saw, such as `localhost:5432 refused`.
    pub saw: String,
    pub proved_in: Option<String>,
    pub hint: Option<String>,
}

impl Skip {
    /// One line: what wasn't run, the need and what its check saw, and
    /// where it's proved or what to do.
    pub fn line(&self) -> String {
        let what = match self.files {
            0 => "not run here".to_string(),
            1 => "1 file not run here".to_string(),
            n => format!("{n} files not run here"),
        };
        let head = format!(
            "{}: {what}: need \"{}\" not met ({})",
            self.step, self.need, self.saw
        );
        match (&self.proved_in, &self.hint) {
            (Some(at), Some(hint)) => format!("{head}; proved in {at}. To run them here: {hint}"),
            (Some(at), None) => format!("{head}; proved in {at}"),
            (None, hint) => format!(
                "{head}, and the need names no `proved_in`, so the gate can't pass. Meet it{} or set its `proved_in` to where these are proved, then run `fairlead done` again",
                hint.as_ref().map(|h| format!(" ({h})")).unwrap_or_default()
            ),
        }
    }

    pub fn record(&self) -> Unmet {
        Unmet {
            need: self.need.clone(),
            step: self.step.clone(),
            files: self.files,
            proved: self.proved_in.is_some(),
        }
    }
}

/// The steps with what an unmet need covers left out, and what was left
/// out. Only needs some step touches are checked, each once.
pub fn apply(
    planned: &Planned,
    steps: Vec<Invocation>,
    root: &Path,
) -> Result<(Vec<Invocation>, Vec<Skip>), String> {
    let needs = planned.config.needs.items();
    if needs.is_empty() {
        return Ok((steps, Vec::new()));
    }
    let globs = needs
        .iter()
        .map(|n| n.tests.iter().map(|g| Pattern::new(g)).collect())
        .collect::<Result<Vec<Vec<Pattern>>, String>>()?;
    let tests: Vec<Vec<String>> = steps.iter().map(|s| tests_of(planned, s)).collect();
    let touches = |i: usize| {
        steps.iter().zip(&tests).any(|(s, t)| match s.kind {
            InvocationKind::Check => needs[i].checks.contains(&s.id),
            InvocationKind::Runner => t.iter().any(|p| globs[i].iter().any(|g| g.is_match(p))),
        })
    };
    let touched: Vec<usize> = (0..needs.len()).filter(|&i| touches(i)).collect();
    let subset: Vec<_> = touched.iter().map(|&i| needs[i].clone()).collect();
    let checked = crate::needs::check_all(&subset, root);
    let unmet: Vec<(usize, String)> = touched
        .iter()
        .zip(checked)
        .filter(|(_, c)| !c.met)
        .map(|(&i, c)| (i, c.saw))
        .collect();
    if unmet.is_empty() {
        return Ok((steps, Vec::new()));
    }
    let skip = |i: usize, saw: &str, step: &str, files: usize| Skip {
        need: needs[i].id.clone(),
        step: step.to_string(),
        files,
        saw: saw.to_string(),
        proved_in: needs[i].proved_in.clone(),
        hint: needs[i].hint.clone(),
    };
    let cover = |path: &str| {
        unmet
            .iter()
            .find(|(i, _)| globs[*i].iter().any(|g| g.is_match(path)))
    };
    let mut kept = Vec::new();
    let mut skips = Vec::new();
    for (mut step, tests) in steps.into_iter().zip(tests) {
        if step.kind == InvocationKind::Check {
            match unmet
                .iter()
                .find(|(i, _)| needs[*i].checks.contains(&step.id))
            {
                Some((i, saw)) => skips.push(skip(*i, saw, &step.id, 0)),
                None => kept.push(step),
            }
            continue;
        }
        let mut by_need: BTreeMap<usize, (String, usize)> = BTreeMap::new();
        for path in &tests {
            if let Some((i, saw)) = cover(path) {
                by_need.entry(*i).or_insert_with(|| (saw.clone(), 0)).1 += 1;
            }
        }
        let left_out: usize = by_need.values().map(|(_, n)| n).sum();
        if left_out == 0 {
            kept.push(step);
            continue;
        }
        let named = is_named(&step, &tests);
        // A suite run whole can't leave a file out: it waits only when the need covers all of it.
        if !named && left_out < tests.len() {
            kept.push(step);
            continue;
        }
        skips.extend(
            by_need
                .iter()
                .map(|(i, (saw, n))| skip(*i, saw, &step.id, *n)),
        );
        if named && left_out < tests.len() {
            step.argv
                .retain(|a| repo_path(&step.cwd, a).is_none_or(|p| cover(&p).is_none()));
            kept.push(step);
        }
    }
    Ok((kept, skips))
}

/// The test files a runner step runs: those its argv names, or for a step
/// that names none, every planned test of its runner under its directory.
fn tests_of(planned: &Planned, step: &Invocation) -> Vec<String> {
    if step.kind == InvocationKind::Check {
        return Vec::new();
    }
    let mine: Vec<&str> = planned
        .plan
        .tests
        .iter()
        .filter(|t| t.runner.as_deref() == Some(step.id.as_str()))
        .map(|t| t.path.as_str())
        .collect();
    let named: Vec<String> = step
        .argv
        .iter()
        .filter_map(|a| repo_path(&step.cwd, a))
        .filter(|p| mine.contains(&p.as_str()))
        .collect();
    if !named.is_empty() {
        return named;
    }
    let under = match step.cwd.as_str() {
        "." | "" => String::new(),
        dir => format!("{dir}/"),
    };
    mine.into_iter()
        .filter(|p| p.starts_with(&under))
        .map(String::from)
        .collect()
}

fn is_named(step: &Invocation, tests: &[String]) -> bool {
    step.argv
        .iter()
        .filter_map(|a| repo_path(&step.cwd, a))
        .any(|p| tests.contains(&p))
}

/// An argument as a repository path, read from the step's directory.
fn repo_path(cwd: &str, arg: &str) -> Option<String> {
    let dir = if cwd == "." { "" } else { cwd };
    fairlead_lang::tree::normalize(dir, arg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skip_says_where_it_is_proved_or_what_to_do() {
        let mut s = Skip {
            need: "postgres".into(),
            step: "api".into(),
            files: 5,
            saw: "localhost:5432 refused".into(),
            proved_in: Some("CI (api job)".into()),
            hint: Some("docker compose up -d db".into()),
        };
        assert_eq!(
            s.line(),
            "api: 5 files not run here: need \"postgres\" not met (localhost:5432 refused); proved in CI (api job). To run them here: docker compose up -d db"
        );
        s.proved_in = None;
        assert!(s.line().ends_with("Meet it (docker compose up -d db) or set its `proved_in` to where these are proved, then run `fairlead done` again"));
        assert_eq!(
            repo_path("api", "test/a.test.ts").as_deref(),
            Some("api/test/a.test.ts")
        );
        assert_eq!(
            repo_path(".", "test/a.test.ts").as_deref(),
            Some("test/a.test.ts")
        );
    }
}
