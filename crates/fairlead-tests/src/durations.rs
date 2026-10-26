//! Seconds per test file, which balance a plan's shards: a JSON object of
//! path to seconds, or a JUnit XML report. A report's `testcase` times are
//! summed per file, the file read from the test case's `file` attribute,
//! else its `testsuite`'s, else a `classname` holding a `/`; a suite with a
//! file whose test cases carry no time counts its own `time`. Several files
//! add up. Paths are kept relative to the repository root.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Default)]
pub struct Durations {
    seconds: BTreeMap<String, f64>,
}

impl Durations {
    /// Adds the times in one file's text; `name` is for errors only.
    pub fn add(&mut self, root: &Path, name: &str, text: &str) -> Result<(), String> {
        let text = text.trim_start_matches('\u{feff}').trim_start();
        let found = match text.chars().next() {
            Some('{') => from_json(name, text)?,
            Some('<') => from_junit(text),
            _ => {
                return Err(format!(
                    "{name} is neither a JSON object of path to seconds nor a JUnit XML report"
                ))
            }
        };
        let root = root.to_string_lossy().replace('\\', "/");
        for (path, seconds) in found {
            *self.seconds.entry(normalize(&root, &path)).or_default() += seconds;
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.seconds.is_empty()
    }

    /// A test file's seconds, by its repository path or by its path from
    /// the directory its runner runs in.
    pub fn get(&self, path: &str, cwd: &str) -> Option<f64> {
        if let Some(s) = self.seconds.get(path) {
            return Some(*s);
        }
        let cwd = cwd.trim_start_matches("./").trim_end_matches('/');
        if cwd.is_empty() || cwd == "." {
            return None;
        }
        let rest = path.strip_prefix(cwd)?.strip_prefix('/')?;
        self.seconds.get(rest).copied()
    }
}

/// Forward slashes, no leading `./`, and an absolute path inside the
/// repository made relative to it.
fn normalize(root: &str, path: &str) -> String {
    let path = path.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let path = match path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
        Some(rest) if !root.is_empty() => rest.to_string(),
        _ => path,
    };
    let mut path = path.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    path.to_string()
}

fn from_json(name: &str, text: &str) -> Result<Vec<(String, f64)>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("{name} isn't JSON: {e}"))?;
    let map = value
        .as_object()
        .ok_or_else(|| format!("{name} isn't a JSON object of path to seconds"))?;
    map.iter()
        .map(|(path, v)| match v.as_f64() {
            Some(s) if s.is_finite() && s >= 0.0 => Ok((path.clone(), s)),
            _ => Err(format!("{name}: `{path}` isn't a number of seconds")),
        })
        .collect()
}

struct Suite {
    file: Option<String>,
    time: Option<f64>,
    /// A test case under it, or a nested suite, already counted a time.
    counted: bool,
}

fn from_junit(text: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let mut suites: Vec<Suite> = Vec::new();
    for tag in tags(text) {
        match (tag.name, tag.kind) {
            ("testsuite", Kind::Open | Kind::Empty) => {
                let inherited = suites.last().and_then(|s| s.file.clone());
                let suite = Suite {
                    file: attr(tag.attrs, "file").or(inherited),
                    time: attr(tag.attrs, "time").and_then(|t| seconds(&t)),
                    counted: false,
                };
                if tag.kind == Kind::Open {
                    suites.push(suite);
                } else if let (Some(file), Some(time)) = (suite.file, suite.time) {
                    out.push((file, time));
                }
            }
            ("testsuite", Kind::Close) => {
                let Some(suite) = suites.pop() else { continue };
                let mut counted = suite.counted;
                if !counted {
                    if let (Some(file), Some(time)) = (suite.file, suite.time) {
                        out.push((file, time));
                        counted = true;
                    }
                }
                if let Some(parent) = suites.last_mut() {
                    parent.counted |= counted;
                }
            }
            ("testcase", Kind::Open | Kind::Empty) => {
                let file = attr(tag.attrs, "file")
                    .or_else(|| suites.last().and_then(|s| s.file.clone()))
                    .or_else(|| attr(tag.attrs, "classname").filter(|c| c.contains('/')));
                let time = attr(tag.attrs, "time").and_then(|t| seconds(&t));
                if let (Some(file), Some(time)) = (file, time) {
                    out.push((file, time));
                    if let Some(suite) = suites.last_mut() {
                        suite.counted = true;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn seconds(text: &str) -> Option<f64> {
    let s: f64 = text.trim().replace(',', "").parse().ok()?;
    (s.is_finite() && s >= 0.0).then_some(s)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open,
    Close,
    Empty,
}

struct Tag<'a> {
    name: &'a str,
    kind: Kind,
    attrs: &'a str,
}

/// The element tags in order, past comments, CDATA, declarations and
/// processing instructions; a `>` inside a quoted value doesn't end a tag.
fn tags(text: &str) -> Vec<Tag<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = text[at..].find('<').map(|i| at + i) {
        let rest = &text[start..];
        let skip_to = |end: &str| rest.find(end).map(|i| start + i + end.len());
        let next = if rest.starts_with("<!--") {
            skip_to("-->")
        } else if rest.starts_with("<![CDATA[") {
            skip_to("]]>")
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            skip_to(">")
        } else {
            let end = tag_end(rest).map(|i| start + i);
            if let Some(end) = end {
                out.extend(tag(&text[start + 1..end]));
            }
            end.map(|e| e + 1)
        };
        match next {
            Some(n) => at = n,
            None => break,
        }
    }
    out
}

fn tag_end(rest: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in rest.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn tag(inner: &str) -> Option<Tag<'_>> {
    let (kind, body) = if let Some(b) = inner.strip_prefix('/') {
        (Kind::Close, b)
    } else if let Some(b) = inner.strip_suffix('/') {
        (Kind::Empty, b)
    } else {
        (Kind::Open, inner)
    };
    let split = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
    let name = &body[..split];
    (!name.is_empty()).then_some(Tag {
        name,
        kind,
        attrs: &body[split..],
    })
}

/// An attribute's value, entities decoded.
fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let close = after[1..].find(quote)? + 1;
        if name == key {
            return Some(unescape(&after[1..close]));
        }
        rest = &after[close + 1..];
    }
}

fn unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Durations {
        let mut d = Durations::default();
        d.add(Path::new("/repo"), "durations", text).unwrap();
        d
    }

    #[test]
    fn a_json_map_reads_as_seconds_per_path() {
        let d = read(r#"{"test/a.test.ts": 12.5, "./test/b.test.ts": 3}"#);
        assert_eq!(d.get("test/a.test.ts", "."), Some(12.5));
        assert_eq!(d.get("test/b.test.ts", "."), Some(3.0));
        assert_eq!(d.get("test/c.test.ts", "."), None);
    }

    #[test]
    fn a_json_value_that_isnt_seconds_is_an_error() {
        let mut d = Durations::default();
        let err = d
            .add(Path::new("/repo"), "d.json", r#"{"a": "slow"}"#)
            .unwrap_err();
        assert!(err.contains("`a` isn't a number of seconds"), "{err}");
        assert!(d.add(Path::new("/repo"), "d.json", "[1]").is_err());
        assert!(d.add(Path::new("/repo"), "d.txt", "a 1").is_err());
    }

    #[test]
    fn junit_sums_test_cases_per_file_from_each_place_a_file_can_be_named() {
        let d = read(
            r#"<?xml version="1.0"?>
<!-- a <testcase> in a comment is not one -->
<testsuites>
  <testsuite name="a" file="test/a.test.ts" time="9">
    <testcase name="one" time="1.5"/>
    <testcase name="two" time="2"><failure><![CDATA[<testcase time="99">]]></failure></testcase>
  </testsuite>
  <testsuite name="b">
    <testcase name="x" file="/repo/test/b.test.ts" time="4"/>
    <testcase name="y" classname="test/c.test.ts" time="1,000.5"/>
    <testcase name="z" classname="pkg.Module" time="7"/>
  </testsuite>
  <testsuite name="d" file="test/d.test.ts" time="6"></testsuite>
  <testsuite name="e" file="test/e.test.ts" time="2"/>
</testsuites>"#,
        );
        assert_eq!(d.get("test/a.test.ts", "."), Some(3.5));
        assert_eq!(d.get("test/b.test.ts", "."), Some(4.0));
        assert_eq!(d.get("test/c.test.ts", "."), Some(1000.5));
        assert_eq!(d.get("test/d.test.ts", "."), Some(6.0));
        assert_eq!(d.get("test/e.test.ts", "."), Some(2.0));
        assert_eq!(d.get("pkg.Module", "."), None);
    }

    #[test]
    fn a_path_from_the_runners_directory_matches_and_files_add_up() {
        let mut d = read(r#"{"test/a.test.ts": 2}"#);
        d.add(Path::new("/repo"), "more", r#"{"test/a.test.ts": 1}"#)
            .unwrap();
        assert_eq!(d.get("apps/web/test/a.test.ts", "apps/web"), Some(3.0));
        assert_eq!(d.get("apps/api/test/a.test.ts", "apps/web"), None);
        assert_eq!(d.get("test/a.test.ts", "."), Some(3.0));
    }

    #[test]
    fn a_quoted_angle_bracket_and_entities_stay_in_the_value() {
        let d = read(
            r#"<testsuite><testcase name="a > b" file="test/&quot;q&quot;.ts" time="1"/></testsuite>"#,
        );
        assert_eq!(d.get("test/\"q\".ts", "."), Some(1.0));
    }
}
