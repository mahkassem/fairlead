//! The fixture corpus: one small file per edge case of each preset rule,
//! checked against findings written once from a reference implementation
//! of the same rules and frozen in `comments.expected`.

use std::path::{Path, PathBuf};

use fairlead_guard::{Guard, Source};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn the_fixture_corpus_gives_exactly_the_frozen_findings() {
    let config =
        fairlead_core::config::load_file(&Path::new(FIXTURES).join("comments.toml"), &[]).unwrap();
    assert!(config.problems.is_empty(), "{:?}", config.problems);
    let guard = Guard::new(&config.config.guard, Path::new(FIXTURES)).unwrap();
    let root = Path::new(FIXTURES).join("comments");
    let mut found = Vec::new();
    for path in files(&root) {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&path).unwrap();
        for f in guard.lint(&Source::new(&rel, &text)) {
            found.push(format!("{}:{} {}", f.file, f.line, f.rule));
        }
    }
    found.sort();
    let expected = std::fs::read_to_string(Path::new(FIXTURES).join("comments.expected")).unwrap();
    let expected: Vec<&str> = expected.lines().collect();
    assert_eq!(found, expected);
}
