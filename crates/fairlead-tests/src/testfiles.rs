//! Test files: which files are tests, which runner runs each one, and which
//! class decides when it's selected. A test with no runner, or with two, is
//! a config problem named with the files it hits.

use fairlead_core::config::{Config, Runner, TestClass};
use fairlead_lang::tree::Tree;

use crate::modules::Modules;
use crate::pattern::Pattern;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestFile {
    pub path: String,
    pub module: Option<usize>,
    /// Index into the config's runners; `None` when none are configured.
    pub runner: Option<usize>,
    pub class: TestClass,
}

#[derive(Debug, Default)]
pub struct Discovered {
    pub tests: Vec<TestFile>,
    /// Test files no runner matches, when runners are configured.
    pub unmatched: Vec<String>,
    /// Test files two or more runners match, with the runner ids.
    pub ambiguous: Vec<(String, Vec<String>)>,
}

fn compile(globs: &[String]) -> Result<Vec<Pattern>, String> {
    globs.iter().map(|g| Pattern::new(g)).collect()
}

fn any(patterns: &[Pattern], path: &str) -> bool {
    patterns.iter().any(|p| p.is_match(path))
}

/// Test files that don't map to exactly one runner, one line each; empty
/// when no runners are configured.
pub fn runner_problems(tree: &Tree, config: &Config) -> Result<Vec<String>, String> {
    if config.tests.runners.items().is_empty() {
        return Ok(Vec::new());
    }
    let found = discover(tree, config, &Modules::default())?;
    let mut out: Vec<String> = found
        .unmatched
        .iter()
        .map(|p| format!("{p}: no [[tests.runners]] matches it"))
        .collect();
    out.extend(
        found
            .ambiguous
            .iter()
            .map(|(p, ids)| format!("{p}: matched by more than one runner ({})", ids.join(", "))),
    );
    Ok(out)
}

pub fn discover(tree: &Tree, config: &Config, modules: &Modules) -> Result<Discovered, String> {
    let include = compile(config.tests.matches.items())?;
    let exclude = compile(config.tests.exclude.items())?;
    let runners: Vec<(&Runner, Vec<Pattern>, Vec<Pattern>)> = config
        .tests
        .runners
        .items()
        .iter()
        .map(|r| Ok((r, compile(&r.matches)?, compile(&r.exclude)?)))
        .collect::<Result<_, String>>()?;
    let classes: Vec<(TestClass, Vec<Pattern>)> = config
        .tests
        .classes
        .items()
        .iter()
        .map(|c| compile(&c.matches).map(|p| (c.class, p)))
        .collect::<Result<_, _>>()?;
    let mut found = Discovered::default();
    for path in &tree.files {
        if !any(&include, path) || any(&exclude, path) {
            continue;
        }
        let matching: Vec<usize> = runners
            .iter()
            .enumerate()
            .filter(|(_, (_, p, x))| any(p, path) && !any(x, path))
            .map(|(i, _)| i)
            .collect();
        match matching.len() {
            0 if !runners.is_empty() => found.unmatched.push(path.clone()),
            0 | 1 => {}
            _ => found.ambiguous.push((
                path.clone(),
                matching.iter().map(|&i| runners[i].0.id.clone()).collect(),
            )),
        }
        let class = classes
            .iter()
            .find(|(_, p)| any(p, path))
            .map_or(TestClass::Unit, |(c, _)| *c);
        found.tests.push(TestFile {
            path: path.clone(),
            module: modules.of(path),
            runner: matching.first().copied(),
            class,
        });
    }
    Ok(found)
}
