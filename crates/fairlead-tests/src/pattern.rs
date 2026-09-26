//! Globs that can capture a path segment: `services/{name}/src/**` matches
//! `services/api/src/x.ts` with `name = "api"`. Owner rules and module
//! patterns need the capture, which `globset` can't give, so these compile
//! to an anchored regex. `{a,b}` with a comma is still an alternation.

use std::collections::BTreeMap;

use regex::Regex;

#[derive(Debug, Clone)]
pub struct Pattern {
    source: String,
    regex: Regex,
}

pub type Captures = BTreeMap<String, String>;

impl Pattern {
    pub fn new(glob: &str) -> Result<Pattern, String> {
        let body = translate(glob)?;
        let regex = Regex::new(&format!("^{body}$")).map_err(|e| format!("`{glob}`: {e}"))?;
        Ok(Pattern {
            source: glob.to_string(),
            regex,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_match(&self, path: &str) -> bool {
        self.regex.is_match(path)
    }

    /// The captured segments when `path` matches.
    pub fn captures(&self, path: &str) -> Option<Captures> {
        let found = self.regex.captures(path)?;
        Some(
            self.regex
                .capture_names()
                .flatten()
                .filter_map(|n| Some((n.to_string(), found.name(n)?.as_str().to_string())))
                .collect(),
        )
    }
}

/// `glob` with each `{name}` replaced by its captured value.
pub fn fill(glob: &str, captures: &Captures) -> String {
    let mut out = glob.to_string();
    for (name, value) in captures {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn is_capture(inner: &str) -> bool {
    !inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn translate(glob: &str) -> Result<String, String> {
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                let at_start = i == 0 || chars[i - 1] == '/';
                if at_start && chars.get(i + 2) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    out.push_str(".*");
                    i += 2;
                }
            }
            '*' => {
                out.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                out.push_str("[^/]");
                i += 1;
            }
            '{' => {
                let close = chars[i..]
                    .iter()
                    .position(|&c| c == '}')
                    .ok_or_else(|| format!("`{glob}`: unclosed `{{`"))?;
                let inner: String = chars[i + 1..i + close].iter().collect();
                if is_capture(&inner) {
                    out.push_str(&format!("(?P<{inner}>[^/]+)"));
                } else {
                    let alternatives: Result<Vec<String>, String> =
                        inner.split(',').map(translate).collect();
                    out.push_str(&format!("(?:{})", alternatives?.join("|")));
                }
                i += close + 1;
            }
            '[' => {
                let close = chars[i..]
                    .iter()
                    .position(|&c| c == ']')
                    .ok_or_else(|| format!("`{glob}`: unclosed `[`"))?;
                let class: String = chars[i + 1..i + close].iter().collect();
                let class = class
                    .strip_prefix('!')
                    .map_or(class.clone(), |c| format!("^{c}"));
                out.push_str(&format!("[{class}]"));
                i += close + 1;
            }
            c => {
                out.push_str(&regex::escape(&c.to_string()));
                i += 1;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(glob: &str, path: &str) -> bool {
        Pattern::new(glob).unwrap().is_match(path)
    }

    #[test]
    fn stars_stay_in_a_segment_and_double_stars_cross_them() {
        assert!(m("src/*.ts", "src/a.ts"));
        assert!(!m("src/*.ts", "src/x/a.ts"));
        assert!(m("src/**/*.ts", "src/a.ts"));
        assert!(m("src/**/*.ts", "src/x/y/a.ts"));
        assert!(m("**/*.test.ts", "a.test.ts"));
        assert!(m("**/*.test.ts", "p/q/a.test.ts"));
        assert!(m("e2e/**", "e2e/a/b.spec.ts"));
        assert!(!m("e2e/**", "e2ex/a.ts"));
    }

    #[test]
    fn braces_with_commas_alternate_and_without_capture() {
        assert!(m("**/*.{test,spec}.{ts,tsx}", "a/b.spec.tsx"));
        assert!(!m("**/*.{test,spec}.ts", "a/b.unit.ts"));
        let p = Pattern::new("services/{name}/test/**").unwrap();
        let caps = p.captures("services/api/test/a/b.test.ts").unwrap();
        assert_eq!(caps.get("name").map(String::as_str), Some("api"));
        assert!(p.captures("services/a/b/test/x.ts").is_none());
        assert_eq!(fill("services/{name}/src/**", &caps), "services/api/src/**");
    }

    #[test]
    fn literal_characters_are_escaped() {
        assert!(m("a+b/(c).ts", "a+b/(c).ts"));
        assert!(!m("a.ts", "abts"));
    }
}
