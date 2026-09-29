//! Globs that can capture a path segment: `services/{name}/src/**` matches
//! `services/api/src/x.ts` with `name = "api"`. Owner rules and module
//! patterns need the capture, which `globset` can't give, so these compile
//! to an anchored regex. `{a,b}` with a comma is still an alternation, and
//! may nest; a backslash makes the next character literal, so `\[` is `[`.

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
        compile(glob, &Captures::new())
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_match(&self, path: &str) -> bool {
        self.regex.is_match(path)
    }

    /// The names this pattern captures, or none when it captures nothing.
    pub fn captures_names(&self) -> Option<std::collections::BTreeSet<String>> {
        let names: std::collections::BTreeSet<String> = self
            .regex
            .capture_names()
            .flatten()
            .map(String::from)
            .collect();
        (!names.is_empty()).then_some(names)
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

/// `glob` with each `{name}` fixed to its captured value. The value is
/// matched literally, never as glob syntax, so a segment like `[slug]` or
/// `{id}` stands for itself.
pub fn fill(glob: &str, captures: &Captures) -> Result<Pattern, String> {
    compile(glob, captures)
}

fn compile(glob: &str, values: &Captures) -> Result<Pattern, String> {
    let body = translate(glob, values)?;
    let regex = Regex::new(&format!("^{body}$")).map_err(|e| format!("`{glob}`: {e}"))?;
    Ok(Pattern {
        source: glob.to_string(),
        regex,
    })
}

/// Whether `{inner}` is a capture; any other brace group is an alternation.
pub fn is_capture(inner: &str) -> bool {
    !inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn translate(glob: &str, values: &Captures) -> Result<String, String> {
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
            '\\' => {
                let literal = chars.get(i + 1).copied().unwrap_or('\\');
                out.push_str(&regex::escape(&literal.to_string()));
                i += 2;
            }
            '{' => {
                let (close, commas) =
                    group(&chars, i).ok_or_else(|| format!("`{glob}`: unclosed `{{`"))?;
                let inner: String = chars[i + 1..close].iter().collect();
                if is_capture(&inner) {
                    match values.get(&inner) {
                        Some(value) => out.push_str(&regex::escape(value)),
                        None => out.push_str(&format!("(?P<{inner}>[^/]+)")),
                    }
                } else {
                    let mut bounds = vec![i];
                    bounds.extend(&commas);
                    bounds.push(close);
                    let alternatives: Result<Vec<String>, String> = bounds
                        .windows(2)
                        .map(|w| {
                            translate(&chars[w[0] + 1..w[1]].iter().collect::<String>(), values)
                        })
                        .collect();
                    out.push_str(&format!("(?:{})", alternatives?.join("|")));
                }
                i = close + 1;
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

/// The `}` closing the brace group opening at `open`, and the commas at its
/// own depth, skipping escaped characters and `[...]` classes.
fn group(chars: &[char], open: usize) -> Option<(usize, Vec<usize>)> {
    let mut depth = 0;
    let mut commas = Vec::new();
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' => i += chars[i..].iter().position(|&c| c == ']')?,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((i, commas));
                }
            }
            ',' if depth == 1 => commas.push(i),
            _ => {}
        }
        i += 1;
    }
    None
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
        let filled = fill("services/{name}/src/**", &caps).unwrap();
        assert!(filled.is_match("services/api/src/x.ts"));
        assert!(!filled.is_match("services/web/src/x.ts"));
    }

    #[test]
    fn a_filled_value_is_matched_literally_not_as_glob_syntax() {
        for value in ["[slug]", "{id}", "*", "a?b", "{a,b}"] {
            let caps = Captures::from([("name".to_string(), value.to_string())]);
            let filled = fill("pages/{name}/**", &caps).unwrap();
            assert!(filled.is_match(&format!("pages/{value}/x.ts")), "{value}");
            assert!(!filled.is_match("pages/s/x.ts"), "{value}");
            assert!(!filled.is_match("pages/other/x.ts"), "{value}");
        }
    }

    #[test]
    fn alternations_nest_and_a_backslash_makes_the_next_character_literal() {
        let nested = "{src/**/*.test.{ts,tsx},tests/**/*.test.ts}";
        assert!(m(nested, "src/a/b.test.tsx"));
        assert!(m(nested, "tests/c.test.ts"));
        assert!(!m(nested, "tests/c.test.tsx"));
        assert!(m("app/**/\\[...slug\\]/**", "app/docs/[...slug]/page.tsx"));
        assert!(!m("app/**/\\[...slug\\]/**", "app/docs/s/page.tsx"));
        assert!(m("a\\{b\\}.ts", "a{b}.ts"));
        assert!(m("{a\\,b,c}.ts", "a,b.ts"));
        assert!(Pattern::new("{a,{b,c}").is_err());
    }

    #[test]
    fn literal_characters_are_escaped() {
        assert!(m("a+b/(c).ts", "a+b/(c).ts"));
        assert!(!m("a.ts", "abts"));
    }
}
