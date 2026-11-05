//! `[[needs]]`: what some tests need from the machine, such as a service on a
//! port or a container runtime, and how to check for it (#259). The check
//! looks at the need itself, never at whether Fairlead runs in a container.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::validate::{globs, problem};
use super::{Config, Problem};

/// A need, checked one way: `tcp`, `command` or `env`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Need {
    /// Lowercase letters, digits and dashes, such as "smtp".
    pub id: String,
    /// Addresses as `host:port`; the need is met when any one accepts a
    /// connection. `{env:NAME}` is replaced by that variable, and an address
    /// naming an unset one is skipped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tcp: Vec<String>,
    /// An argv that exits 0 when the need is met, such as `["docker", "info"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// A variable that is set and not empty when the need is met.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// Test files that need it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    /// `[[checks]]` ids that need it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
    /// Where its tests are proved instead, such as "CI (services job)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proved_in: Option<String>,
    /// How to meet it on this machine, in a sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// How long one try may take, in milliseconds: 300 for `tcp` and 2000 for
    /// `command` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl Need {
    /// The timeout in force for one try.
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
            .unwrap_or(if self.command.is_empty() { 300 } else { 2000 })
    }
}

/// The `{env:NAME}` placeholders in an address, or why one is malformed.
pub fn env_names(address: &str) -> Result<Vec<&str>, String> {
    let mut names = Vec::new();
    let mut rest = address;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("`{address}` has a `{{` with no `}}`"))?;
        let inner = &after[..end];
        let name = inner
            .strip_prefix("env:")
            .filter(|n| is_env_name(n))
            .ok_or_else(|| format!("`{{{inner}}}` isn't `{{env:NAME}}`"))?;
        names.push(name);
        rest = &after[end + 1..];
    }
    Ok(names)
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub(super) fn validate(config: &Config, problems: &mut Vec<Problem>) {
    let needs = config.needs.items();
    let mut seen = std::collections::BTreeSet::new();
    for (i, need) in needs.iter().enumerate() {
        let key = format!("needs[{i}]");
        if !is_id(&need.id) {
            problems.push(problem(
                format!("{key}.id"),
                "must be lowercase letters, digits and dashes",
            ));
        } else if !seen.insert(need.id.as_str()) {
            problems.push(problem(
                "needs",
                format!("need id `{}` is used more than once", need.id),
            ));
        }
        let kinds = [
            !need.tcp.is_empty(),
            !need.command.is_empty(),
            need.env.is_some(),
        ];
        if kinds.iter().filter(|k| **k).count() != 1 {
            problems.push(problem(
                key.clone(),
                "needs exactly one of `tcp`, `command` or `env`",
            ));
        }
        for (j, address) in need.tcp.iter().enumerate() {
            if let Err(e) = check_address(address) {
                problems.push(problem(format!("{key}.tcp[{j}]"), e));
            }
        }
        if need.env.as_deref().is_some_and(|n| !is_env_name(n)) {
            problems.push(problem(format!("{key}.env"), "isn't a variable name"));
        }
        if need.tests.is_empty() && need.checks.is_empty() {
            problems.push(problem(
                key.clone(),
                "needs `tests` or `checks`, or nothing needs it",
            ));
        }
        globs(&format!("{key}.tests"), &need.tests, problems);
        let ids: Vec<&str> = config
            .checks
            .items()
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        for (j, check) in need.checks.iter().enumerate() {
            if !ids.contains(&check.as_str()) {
                problems.push(problem(
                    format!("{key}.checks[{j}]"),
                    format!("no `[[checks]]` has id `{check}`"),
                ));
            }
        }
        if need.timeout_ms.is_some_and(|t| t == 0 || t > 30_000) {
            problems.push(problem(
                format!("{key}.timeout_ms"),
                "is milliseconds, from 1 to 30000",
            ));
        }
    }
}

/// `host:port`, where either part may hold `{env:NAME}`; a literal port must
/// be a number from 1 to 65535.
fn check_address(address: &str) -> Result<(), String> {
    env_names(address)?;
    let (host, port) = address
        .rsplit_once(':')
        .ok_or_else(|| format!("`{address}` isn't `host:port`"))?;
    if host.is_empty() {
        return Err(format!("`{address}` has no host"));
    }
    if !port.contains('{') && !matches!(port.parse::<u16>(), Ok(p) if p > 0) {
        return Err(format!("`{port}` isn't a port"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn need(toml: &str) -> Vec<Problem> {
        let config: Config = toml::from_str(toml).expect("parses");
        let mut problems = Vec::new();
        validate(&config, &mut problems);
        problems
    }

    #[test]
    fn a_need_with_one_check_and_tests_is_valid() {
        let found = need(
            "[[needs]]\nid = \"smtp\"\ntcp = [\"localhost:1025\", \"{env:MAIL_HOST}:1025\"]\ntests = [\"tests/Mail/**\"]\n",
        );
        assert_eq!(found, vec![]);
    }

    #[test]
    fn a_need_with_no_check_or_two_is_refused() {
        let none = need("[[needs]]\nid = \"a\"\ntests = [\"t/**\"]\n");
        let two =
            need("[[needs]]\nid = \"a\"\nenv = \"X\"\ncommand = [\"true\"]\ntests = [\"t/**\"]\n");
        for found in [none, two] {
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].message.contains("exactly one"));
        }
    }

    #[test]
    fn bad_addresses_ids_and_references_are_named() {
        let found = need(
            "[[needs]]\nid = \"Mail\"\ntcp = [\"localhost\", \"h:0\", \"{MAIL}:25\"]\nchecks = [\"e2e\"]\ntimeout_ms = 0\n",
        );
        let keys: Vec<&str> = found.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "needs[0].id",
                "needs[0].tcp[0]",
                "needs[0].tcp[1]",
                "needs[0].tcp[2]",
                "needs[0].checks[0]",
                "needs[0].timeout_ms"
            ]
        );
    }

    #[test]
    fn env_names_are_read_from_an_address() {
        assert_eq!(env_names("{env:H}:{env:P}"), Ok(vec!["H", "P"]));
        assert!(env_names("{env:1X}:1").is_err());
        assert!(env_names("{env:H:1").is_err());
    }
}
