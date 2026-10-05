//! `[[quarantine]]` on the machine at hand: the conditions detected here,
//! the entries that hold, and whether a failure is the one an entry expects.

use std::path::Path;

use fairlead_core::config::{Condition, Config, PlatformQuarantine};
use fairlead_core::plan::{CheckSelection, InvocationKind, Quarantined, TestSelection, Warning};

use crate::planner::Context;

/// The machine a plan is made on: its OS, the conditions detected in the
/// repository, and the day, which ends an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Here {
    pub os: &'static str,
    pub conditions: Vec<Condition>,
    pub today: String,
}

impl Here {
    pub fn detect(root: &Path) -> Here {
        let mut conditions = Vec::new();
        if crate::git::config_true(root, "core.autocrlf") {
            conditions.push(Condition::Autocrlf);
        }
        let abs = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if abs.to_string_lossy().contains(' ') {
            conditions.push(Condition::SpaceInPath);
        }
        Here {
            os: std::env::consts::OS,
            conditions,
            today: fairlead_core::coverage::today(),
        }
    }

    /// The OS, then each condition detected, such as `windows, autocrlf`.
    pub fn describe(&self) -> String {
        std::iter::once(self.os)
            .chain(self.conditions.iter().map(|c| c.name()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The entry holds here, for these reasons: its OS and conditions.
    Holds(Vec<String>),
    /// It would hold, but its date has passed.
    Expired,
    /// It doesn't hold here, and why.
    Elsewhere(String),
}

pub fn status(entry: &PlatformQuarantine, here: &Here) -> Status {
    if let Some(os) = entry.os.filter(|os| os.name() != here.os) {
        return Status::Elsewhere(format!("it names {}, and this is {}", os.name(), here.os));
    }
    let missing: Vec<&str> = entry
        .when
        .iter()
        .filter(|c| !here.conditions.contains(c))
        .map(|c| c.name())
        .collect();
    if !missing.is_empty() {
        return Status::Elsewhere(format!("{} not detected here", missing.join(" and ")));
    }
    if entry.until < here.today {
        return Status::Expired;
    }
    let reasons = entry.os.map(|os| os.name().to_string()).into_iter();
    Status::Holds(
        reasons
            .chain(entry.when.iter().map(|c| c.name().into()))
            .collect(),
    )
}

/// The selected tests and checks an entry holds here. A held test runs
/// alone, so its runner has to take `{files}`; one that can't is left to
/// count, with a warning, as is one whose entry has ended.
pub fn apply(
    cx: &Context,
    tests: &[TestSelection],
    checks: &[CheckSelection],
    warnings: &mut Vec<Warning>,
) -> Vec<Quarantined> {
    let selected = |e: &&PlatformQuarantine| match (&e.path, &e.check) {
        (Some(path), _) => tests.iter().any(|t| &t.path == path),
        (_, Some(id)) => checks.iter().any(|c| &c.id == id),
        _ => false,
    };
    let named: Vec<&PlatformQuarantine> = cx
        .config
        .quarantine
        .items()
        .iter()
        .filter(selected)
        .collect();
    if named.is_empty() {
        return Vec::new();
    }
    let here = Here::detect(&cx.scan.tree.root);
    let mut out = Vec::new();
    for entry in named {
        let target = entry.target().to_string();
        let warn = |code: &str, message: String| Warning {
            code: code.into(),
            path: Some(target.clone()),
            message,
        };
        let reasons = match status(entry, &here) {
            Status::Holds(reasons) => reasons,
            Status::Elsewhere(_) => continue,
            Status::Expired => {
                warnings.push(warn(
                    "quarantine-expired",
                    format!(
                        "its [[quarantine]] entry ended on {}, so a failure here counts",
                        entry.until
                    ),
                ));
                continue;
            }
        };
        if entry.path.is_some() && !separable(cx, tests, &target) {
            warnings.push(warn(
                "quarantine-not-separable",
                "its runner's command has no `{files}`, so it can't run alone and a failure here counts"
                    .into(),
            ));
            continue;
        }
        out.push(held(entry, reasons));
    }
    out
}

/// In a plan that runs everything, a runner holding a test and with no
/// `exclude_arg` names its other tests instead of letting the tool find
/// them, which drops any the tool finds that `match` doesn't claim.
pub fn narrowed(cx: &Context, tests: &[TestSelection], held: &[Quarantined]) -> Vec<Warning> {
    let mut out = Vec::new();
    for runner in cx.config.tests.runners.items() {
        if runner.exclude_arg.is_some() {
            continue;
        }
        let mine: Vec<&str> = tests
            .iter()
            .filter(|t| t.runner.as_deref() == Some(runner.id.as_str()))
            .map(|t| t.path.as_str())
            .filter(|p| {
                held.iter()
                    .any(|q| q.kind == InvocationKind::Runner && q.target == *p)
            })
            .collect();
        let Some(first) = mine.first() else { continue };
        out.push(Warning {
            code: "quarantine-narrowed-everything".into(),
            path: Some((*first).to_string()),
            message: format!(
                "runner `{}` runs {} alone, so its other tests are named instead of found; give it `exclude_arg`, such as [\"--exclude\", \"{{file}}\"], to run the rest whole",
                runner.id,
                if mine.len() == 1 { "this test".to_string() } else { format!("{} held tests", mine.len()) },
            ),
        });
    }
    out
}

/// The entry holding a check here that runs outside the plan, such as one
/// `done.always` names, if one does.
pub fn check_held(config: &Config, root: &Path, id: &str) -> Option<Quarantined> {
    let here = Here::detect(root);
    config
        .quarantine
        .items()
        .iter()
        .filter(|e| e.check.as_deref() == Some(id))
        .find_map(|entry| match status(entry, &here) {
            Status::Holds(reasons) => Some(held(entry, reasons)),
            _ => None,
        })
}

fn held(entry: &PlatformQuarantine, reasons: Vec<String>) -> Quarantined {
    Quarantined {
        target: entry.target().to_string(),
        kind: if entry.path.is_some() {
            InvocationKind::Runner
        } else {
            InvocationKind::Check
        },
        here: reasons,
        signature: entry.signature.clone(),
        reason: entry.reason.clone(),
        proved_in: entry.proved_in.clone(),
        until: entry.until.clone(),
    }
}

/// Whether a selected test's runner can be given it alone.
fn separable(cx: &Context, tests: &[TestSelection], path: &str) -> bool {
    let runner = tests
        .iter()
        .find(|t| t.path == path)
        .and_then(|t| t.runner.as_deref())
        .and_then(|id| cx.config.tests.runners.items().iter().find(|r| r.id == id));
    runner.is_none_or(|r| r.command.iter().any(|a| a == "{files}"))
}

/// Whether a failure that printed `output` is the one `q` expects: its
/// signature matches and, on `today`, its date hasn't passed.
pub fn expected(q: &Quarantined, output: &str, today: &str) -> bool {
    q.until.as_str() >= today && regex::Regex::new(&q.signature).is_ok_and(|re| re.is_match(output))
}

/// One line for a held test or check: why it holds here, the evidence, and
/// where it is proved instead.
pub fn line(q: &Quarantined) -> String {
    format!(
        "[{}] {}; proved in {}, until {}",
        q.here.join(", "),
        q.reason,
        q.proved_in,
        q.until
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use fairlead_core::config::Os;

    fn entry(os: Option<Os>, when: Vec<Condition>, until: &str) -> PlatformQuarantine {
        PlatformQuarantine {
            path: Some("test/a.test.ts".into()),
            check: None,
            os,
            when,
            signature: "ENOENT".into(),
            reason: "spawns git from a URL path".into(),
            proved_in: "CI on Linux".into(),
            until: until.into(),
        }
    }

    fn here(os: &'static str, conditions: Vec<Condition>) -> Here {
        Here {
            os,
            conditions,
            today: "2026-10-01".into(),
        }
    }

    #[test]
    fn an_entry_holds_only_on_its_os_with_every_condition_and_until_its_date() {
        let windows = here("windows", vec![Condition::Autocrlf]);
        let e = entry(Some(Os::Windows), vec![Condition::Autocrlf], "2026-10-01");
        assert_eq!(
            status(&e, &windows),
            Status::Holds(vec!["windows".into(), "autocrlf".into()])
        );
        assert_eq!(
            status(&e, &here("linux", vec![Condition::Autocrlf])),
            Status::Elsewhere("it names windows, and this is linux".into())
        );
        let both = entry(
            None,
            vec![Condition::Autocrlf, Condition::SpaceInPath],
            "2099-01-01",
        );
        assert_eq!(
            status(&both, &windows),
            Status::Elsewhere("space-in-path not detected here".into())
        );
        let ended = entry(Some(Os::Windows), Vec::new(), "2026-09-30");
        assert_eq!(status(&ended, &windows), Status::Expired);
    }

    #[test]
    fn only_a_failure_with_the_signature_before_the_date_is_expected() {
        let q = Quarantined {
            target: "test/a.test.ts".into(),
            kind: InvocationKind::Runner,
            here: vec!["windows".into()],
            signature: r"spawnSync git ENOENT".into(),
            reason: "r".into(),
            proved_in: "CI".into(),
            until: "2026-10-01".into(),
        };
        assert!(expected(
            &q,
            "Error: spawnSync git ENOENT\r\n",
            "2026-10-01"
        ));
        assert!(!expected(&q, "AssertionError: 1 !== 2", "2026-10-01"));
        assert!(!expected(&q, "Error: spawnSync git ENOENT", "2026-10-02"));
    }

    #[test]
    fn a_check_held_on_two_platforms_holds_on_the_second() {
        let all = [Os::Windows, Os::Macos, Os::Linux];
        let this = all
            .iter()
            .copied()
            .find(|o| o.name() == std::env::consts::OS)
            .expect("tests run on a known OS");
        let other = all.iter().copied().find(|o| *o != this).unwrap();
        let check = |os| PlatformQuarantine {
            path: None,
            check: Some("lint".into()),
            ..entry(Some(os), vec![], "2099-12-31")
        };
        let config = Config {
            quarantine: vec![check(other), check(this)].into(),
            ..Config::default()
        };
        let held = check_held(&config, Path::new("."), "lint").expect("the second entry holds");
        assert_eq!(held.here, vec![this.name().to_string()]);
    }
}
