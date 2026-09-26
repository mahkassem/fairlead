//! Owner rules: tests that don't import what they test. A changed path that
//! matches a rule's `covers` selects the tests its `match` names, with any
//! `{name}` the change captured filled in, so `services/api/src/x.ts` picks
//! `services/api/test/**` and not every service's tests.

use fairlead_core::config::Owner;

use crate::pattern::{fill, Pattern};

#[derive(Debug, Clone)]
struct Rule {
    matches: String,
    covers: Vec<Pattern>,
    overrides_run_all: bool,
}

#[derive(Debug, Clone)]
pub struct Owners {
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub rule: usize,
    pub covers: String,
    pub changed: String,
}

impl Owners {
    pub fn new(owners: &[Owner]) -> Result<Owners, String> {
        let rules = owners
            .iter()
            .map(|o| {
                let covers = o
                    .covers
                    .iter()
                    .map(|c| Pattern::new(c))
                    .collect::<Result<_, _>>()?;
                Ok(Rule {
                    matches: o.matches.clone(),
                    covers,
                    overrides_run_all: o.overrides_run_all,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Owners { rules })
    }

    /// Whether any rule covers `path`.
    pub fn covers(&self, path: &str) -> bool {
        self.rules
            .iter()
            .any(|r| r.covers.iter().any(|c| c.is_match(path)))
    }

    /// Whether a rule with `overrides_run_all` covers `path` and claims at
    /// least one of `tests` for it, so a `run_all` match there selects that
    /// rule's tests rather than every test. Claiming none, it runs everything.
    pub fn overrides_run_all(&self, path: &str, tests: &[&str]) -> Result<bool, String> {
        for rule in self.rules.iter().filter(|r| r.overrides_run_all) {
            for cover in &rule.covers {
                let Some(caps) = cover.captures(path) else {
                    continue;
                };
                let claimed = Pattern::new(&fill(&rule.matches, &caps))?;
                if tests.iter().any(|t| claimed.is_match(t)) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// For each test the changes claim, the first claim found.
    pub fn claims<'a>(
        &self,
        changed: impl IntoIterator<Item = &'a str> + Clone,
        tests: &[&str],
    ) -> Result<Vec<(usize, Claim)>, String> {
        let mut out: Vec<(usize, Claim)> = Vec::new();
        for (r, rule) in self.rules.iter().enumerate() {
            for path in changed.clone() {
                for cover in &rule.covers {
                    let Some(caps) = cover.captures(path) else {
                        continue;
                    };
                    let claimed = Pattern::new(&fill(&rule.matches, &caps))?;
                    for (t, test) in tests.iter().enumerate() {
                        if claimed.is_match(test) && !out.iter().any(|(seen, _)| *seen == t) {
                            out.push((
                                t,
                                Claim {
                                    rule: r,
                                    covers: cover.source().to_string(),
                                    changed: path.to_string(),
                                },
                            ));
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_in_covers_narrows_the_tests_claimed() {
        let owners = Owners::new(&[Owner {
            matches: "services/{name}/test/**".into(),
            covers: vec!["services/{name}/src/**".into()],
            overrides_run_all: false,
        }])
        .unwrap();
        let tests = ["services/api/test/a.test.ts", "services/web/test/b.test.ts"];
        let claims = owners.claims(["services/api/src/x.ts"], &tests).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].0, 0);
        assert_eq!(claims[0].1.covers, "services/{name}/src/**");
        assert!(owners.covers("services/web/src/y.ts"));
        assert!(!owners.covers("docs/x.md"));
    }
}
