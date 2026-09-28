//! The coverage map's edges: each test depends on every source file it ran.
//! They join the static graph rather than replace it, so a file the map
//! predates still reaches its tests through its imports.

use std::path::Path;

use fairlead_core::config::Coverage;
use fairlead_core::coverage::{CoverageMap, VERSION};

use crate::graph::{EdgeKind, Graph};

/// What the map added, for `graph stats` and the plan's warnings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub map: String,
    pub commit: String,
    pub created: String,
    pub source: String,
    pub tests: usize,
    pub edges: usize,
    /// Pairs naming a file that isn't in the tree now.
    pub ignored: usize,
    /// Why the map couldn't be read; the plan then has only the static graph.
    pub error: Option<String>,
}

pub fn read(path: &Path) -> Result<CoverageMap, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("couldn't read it: {e}"))?;
    let map: CoverageMap =
        serde_json::from_str(&text).map_err(|e| format!("it isn't a coverage map: {e}"))?;
    if map.version != VERSION {
        return Err(format!(
            "it is version {}; this fairlead reads version {VERSION}",
            map.version
        ));
    }
    Ok(map)
}

pub fn apply(root: &Path, config: &Coverage, graph: &mut Graph) -> Report {
    let mut report = Report {
        map: config.map.clone(),
        ..Report::default()
    };
    let map = match read(&root.join(&config.map)) {
        Ok(m) => m,
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };
    report.commit = map.commit;
    report.created = map.created;
    report.source = map.source;
    report.tests = map.tests.len();
    for (test, files) in &map.tests {
        for file in files {
            match (graph.id(test), graph.id(file)) {
                (Some(from), Some(to)) if from != to => {
                    graph.add_edge(from, to, EdgeKind::Coverage);
                    report.edges += 1;
                }
                (Some(_), Some(_)) => {}
                _ => report.ignored += 1,
            }
        }
    }
    report
}

/// Tests and the files they ran, by repo-relative path.
pub type Tests = std::collections::BTreeMap<String, std::collections::BTreeSet<String>>;

/// A path a coverage run printed, repo-relative: under the root as it is,
/// or, from another checkout, by its longest tail that is a file here.
fn in_tree(tree: &crate::tree::Tree, root: &Path, printed: &str) -> Option<String> {
    let printed = printed.replace('\\', "/");
    let root = root.to_string_lossy().replace('\\', "/");
    let rel = printed
        .strip_prefix(&format!("{}/", root.trim_end_matches('/')))
        .unwrap_or(&printed)
        .trim_start_matches("./");
    if tree.contains(rel) {
        return Some(rel.to_string());
    }
    rel.match_indices('/')
        .map(|(i, _)| &rel[i + 1..])
        .find(|tail| tree.contains(tail))
        .map(str::to_string)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[start..].find('"')? + start;
    Some(
        tag[start..end]
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

/// PHPUnit's `--coverage-xml` directory: each file's report names its path
/// under the project's source and, per line, the `Class::method` tests that
/// ran it; a test class becomes its file through the PHP autoload index.
/// Returns the map and the test names that named no file.
pub fn from_phpunit_xml(
    dir: &Path,
    tree: &crate::tree::Tree,
    autoload: &crate::php::Autoload,
) -> Result<(Tests, Vec<String>), String> {
    let index = std::fs::read_to_string(dir.join("index.xml"))
        .map_err(|e| format!("no index.xml in {}: {e}", dir.display()))?;
    let source = index
        .find("<project ")
        .and_then(|i| attr(&index[i..index[i..].find('>')? + i], "source"))
        .ok_or("index.xml names no project source")?;
    let mut tests = Tests::new();
    let mut unknown = std::collections::BTreeSet::new();
    let mut classes: std::collections::HashMap<String, Option<String>> = Default::default();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "xml")
                || path.file_name() == Some("index.xml".as_ref())
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let Some(file_tag) = text
                .find("<file ")
                .map(|i| &text[i..text[i..].find('>').unwrap_or(0) + i])
            else {
                continue;
            };
            let (Some(name), Some(sub)) = (attr(file_tag, "name"), attr(file_tag, "path")) else {
                continue;
            };
            let printed = format!(
                "{}/{}/{name}",
                source.trim_end_matches('/'),
                sub.trim_matches('/')
            );
            let Some(file) = in_tree(tree, tree.root.as_path(), &printed.replace("//", "/")) else {
                continue;
            };
            for chunk in text.split("<covered by=\"").skip(1) {
                let Some(test) = chunk.split('"').next() else {
                    continue;
                };
                let class = test.split("::").next().unwrap_or(test).to_string();
                let found = classes.entry(class.clone()).or_insert_with(|| {
                    match autoload.resolve(tree, &class) {
                        crate::php::Resolved::Files(f) => f.into_iter().next(),
                        _ => None,
                    }
                });
                match found {
                    Some(test_file) => {
                        tests
                            .entry(test_file.clone())
                            .or_default()
                            .insert(file.clone());
                    }
                    None => {
                        unknown.insert(class);
                    }
                }
            }
        }
    }
    Ok((tests, unknown.into_iter().collect()))
}

/// coverage.py's `coverage json --show-contexts`, from a run with
/// `--cov-context=test`: each context is a pytest node id whose path is
/// the test file. Lines run outside any test have an empty context.
pub fn from_coverage_py(
    text: &str,
    tree: &crate::tree::Tree,
) -> Result<(Tests, Vec<String>), String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not coverage.py JSON: {e}"))?;
    let files = json["files"]
        .as_object()
        .ok_or("no \"files\" in the report")?;
    let mut tests = Tests::new();
    let mut unknown = std::collections::BTreeSet::new();
    let mut any_context = false;
    for (printed, report) in files {
        let Some(file) = in_tree(tree, tree.root.as_path(), printed) else {
            continue;
        };
        let Some(contexts) = report["contexts"].as_object() else {
            continue;
        };
        for ids in contexts.values().filter_map(|v| v.as_array()) {
            for id in ids
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                any_context = true;
                let path = id.split("::").next().unwrap_or(id);
                match in_tree(tree, tree.root.as_path(), path) {
                    Some(test) => {
                        tests.entry(test).or_default().insert(file.clone());
                    }
                    None => {
                        unknown.insert(path.to_string());
                    }
                }
            }
        }
    }
    if !any_context {
        return Err("the report has no test contexts: run pytest with --cov-context=test and export with coverage json --show-contexts".into());
    }
    Ok((tests, unknown.into_iter().collect()))
}
