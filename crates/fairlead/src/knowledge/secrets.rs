//! Secrets and personal data in text that will be committed, such as a
//! lesson. A finding names the kind and the line, never the value.

use std::fmt;
use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: &'static str,
    pub line: usize,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}: looks like {}; committed text must not carry one",
            self.line, self.kind
        )
    }
}

fn patterns() -> &'static [(&'static str, Regex)] {
    static ALL: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    ALL.get_or_init(|| {
        [
            ("a private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
            ("a cloud access key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            ("a GitHub token", r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})"),
            ("a Slack token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}"),
            ("a payment provider key", r"\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}"),
            ("a Google API key", r"\bAIza[0-9A-Za-z_\-]{35}\b"),
            ("a signed token", r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
            (
                "a credential",
                r#"(?i)\b(?:password|passwd|secret|api[_-]?key|access[_-]?key|auth[_-]?token)\b\s*[:=]\s*["']?([^\s"'`]{12,})"#,
            ),
            (
                "an email address",
                r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}\b",
            ),
            ("a card number", r"\b\d(?:[ -]?\d){12,18}\b"),
        ]
        .into_iter()
        .map(|(kind, re)| (kind, Regex::new(re).expect("secret patterns compile")))
        .collect()
    })
}

/// Example and no-reply domains, and git's own user, aren't anybody's address.
fn harmless_email(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let (user, domain) = lower.split_once('@').unwrap_or(("", ""));
    user == "git"
        || [
            "example.com",
            "example.org",
            "example.net",
            "users.noreply.github.com",
        ]
        .iter()
        .any(|d| domain == *d || domain.ends_with(&format!(".{d}")))
        || domain.ends_with(".invalid")
        || domain.ends_with(".test")
}

/// A value that names a secret rather than holding one: an environment
/// variable, a placeholder, or masked text.
fn named_not_held(value: &str) -> bool {
    let v = value.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$');
    v.starts_with('$')
        || v.starts_with('<')
        || v.contains("***")
        || v.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        || !(v.bytes().any(|b| b.is_ascii_digit()) && v.bytes().any(|b| b.is_ascii_alphabetic()))
}

fn luhn(digits: &str) -> bool {
    let ds: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&ds.len()) || ds.iter().all(|&d| d == ds[0]) {
        return false;
    }
    let sum: u32 = ds
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Every line that looks like it carries a secret or personal data.
pub fn scan(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (kind, re) in patterns() {
            let hit = re.captures_iter(line).any(|c| {
                let whole = c.get(0).map_or("", |m| m.as_str());
                match *kind {
                    "an email address" => !harmless_email(whole),
                    "a card number" => luhn(whole),
                    "a credential" => c.get(1).is_some_and(|v| !named_not_held(v.as_str())),
                    _ => true,
                }
            });
            if hit {
                out.push(Finding { kind, line: i + 1 });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<&'static str> {
        scan(text).into_iter().map(|f| f.kind).collect()
    }

    #[test]
    fn each_kind_is_caught_on_its_line() {
        let text = "fine\nkey AKIAABCDEFGHIJKLMNOP here\nmail a.person@company.sa\ncard 4111 1111 1111 1111\n";
        let found = scan(text);
        assert_eq!(
            found[0],
            Finding {
                kind: "a cloud access key",
                line: 2
            }
        );
        assert_eq!(
            found[1],
            Finding {
                kind: "an email address",
                line: 3
            }
        );
        assert_eq!(
            found[2],
            Finding {
                kind: "a card number",
                line: 4
            }
        );
        assert_eq!(kinds("-----BEGIN RSA PRIVATE KEY-----"), ["a private key"]);
        assert_eq!(kinds("password = hunter2hunter2x9"), ["a credential"]);
    }

    #[test]
    fn names_placeholders_and_examples_pass() {
        for ok in [
            "token: `SERVICE_TOKEN` comes from the secret manager",
            "password = $DB_PASSWORD",
            "api_key: <your key here>",
            "secret = ********",
            "write to someone@example.com or git@github.com:org/repo",
            "order 1234 5678 9012 3456 isn't a card",
            "ids 0000000000000000",
        ] {
            assert!(kinds(ok).is_empty(), "{ok}: {:?}", kinds(ok));
        }
    }

    #[test]
    fn a_finding_never_prints_the_value() {
        let f = scan("ghp_abcdefghijklmnopqrstuvwxyz0123456789AB")[0].to_string();
        assert!(f.contains("GitHub token") && !f.contains("ghp_"), "{f}");
    }
}
