//! Workspace packages, read from the manifests rather than an install:
//! `workspaces` in the root `package.json` and `packages` in
//! `pnpm-workspace.yaml`.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde_json::Value;

use crate::tree::{parent, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// Repo-relative directory.
    pub dir: String,
    /// `outDir` and `rootDir` from the package's own tsconfig, relative to
    /// `dir`, when both are set: build output maps back to source through them.
    pub out_to_source: Option<(String, String)>,
}

pub fn discover(tree: &Tree) -> Vec<Package> {
    let patterns = workspace_patterns(tree);
    if patterns.is_empty() {
        return Vec::new();
    }
    let (include, exclude) = globsets(&patterns);
    let mut packages: Vec<Package> = tree
        .files
        .iter()
        .filter(|f| {
            f.ends_with("package.json")
                && (f.as_str() == "package.json" || f.ends_with("/package.json"))
        })
        .map(|f| parent(f))
        .filter(|dir| !dir.is_empty() && include.is_match(dir) && !exclude.is_match(dir))
        .filter_map(|dir| package_at(tree, dir))
        .collect();
    packages.sort_by(|a, b| a.dir.cmp(&b.dir));
    packages
}

fn workspace_patterns(tree: &Tree) -> Vec<String> {
    let mut patterns = Vec::new();
    if let Some(manifest) = read_json(tree, "package.json") {
        let list = match &manifest["workspaces"] {
            Value::Array(items) => items.clone(),
            Value::Object(map) => map
                .get("packages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        patterns.extend(list.iter().filter_map(Value::as_str).map(str::to_string));
    }
    if tree.contains("pnpm-workspace.yaml") {
        if let Ok(text) = std::fs::read_to_string(tree.abs("pnpm-workspace.yaml")) {
            if let Ok(Value::Object(map)) = serde_saphyr::from_str::<Value>(&text) {
                if let Some(Value::Array(items)) = map.get("packages") {
                    patterns.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
                }
            }
        }
    }
    patterns
}

fn globsets(patterns: &[String]) -> (GlobSet, GlobSet) {
    let mut include = GlobSetBuilder::new();
    let mut exclude = GlobSetBuilder::new();
    for pattern in patterns {
        let (target, raw) = match pattern.strip_prefix('!') {
            Some(rest) => (&mut exclude, rest),
            None => (&mut include, pattern.as_str()),
        };
        let cleaned = raw.trim_start_matches("./").trim_end_matches('/');
        // `*` stays within one path segment, as package managers read it.
        if let Ok(glob) = GlobBuilder::new(cleaned).literal_separator(true).build() {
            target.add(glob);
        }
    }
    let empty = || GlobSetBuilder::new().build().expect("empty set builds");
    (
        include.build().unwrap_or_else(|_| empty()),
        exclude.build().unwrap_or_else(|_| empty()),
    )
}

fn package_at(tree: &Tree, dir: &str) -> Option<Package> {
    let manifest = read_json(tree, &format!("{dir}/package.json"))?;
    let name = manifest["name"].as_str()?.to_string();
    let out_to_source = read_json(tree, &format!("{dir}/tsconfig.json")).and_then(|tsconfig| {
        let options = &tsconfig["compilerOptions"];
        let clean = |v: &Value| {
            v.as_str()
                .map(|s| s.trim_start_matches("./").trim_end_matches('/').to_string())
        };
        Some((clean(&options["outDir"])?, clean(&options["rootDir"])?))
    });
    Some(Package {
        name,
        dir: dir.to_string(),
        out_to_source,
    })
}

fn read_json(tree: &Tree, rel: &str) -> Option<Value> {
    if !tree.contains(rel) {
        return None;
    }
    let text = std::fs::read_to_string(tree.abs(rel)).ok()?;
    serde_json::from_str(&strip_jsonc(&text)).ok()
}

/// JSON with comments and trailing commas (tsconfig's dialect) to plain JSON.
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_string) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if c == ',' && next_significant(&chars, i + 1).is_some_and(|n| n == '}' || n == ']')
        {
            // A trailing comma is dropped.
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

fn next_significant(chars: &[char], from: usize) -> Option<char> {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            c if c.is_whitespace() => i += 1,
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            c => return Some(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_comments_and_trailing_commas_become_json() {
        let text = r#"{
  // a comment
  "a": "keep // this", /* block */
  "b": [1, 2, ],
}"#;
        let value: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(value["a"], "keep // this");
        assert_eq!(value["b"], serde_json::json!([1, 2]));
    }
}
