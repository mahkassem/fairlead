//! `ci run --judge`: on a push that runs everything, each failing test file
//! is judged against the merge's own plan. One the plan selected should
//! have failed on the pull request too; one it left out escaped, and the
//! owner rule that would have caught it is the one replay suggests.

use std::path::Path;

use fairlead_core::config::Config;
use fairlead_core::plan::{Invocation, InvocationKind, Plan};
use fairlead_replay::attribute::{attribute, Attribution, Repo};
use fairlead_replay::extract::{extract, Extractor};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub runner: String,
    pub test: String,
    /// `planned` when the merge's plan selected the test, `escaped` when it didn't.
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// What the tree holds, read once for every invocation judged.
pub struct Judge<'a> {
    merge: &'a Plan,
    config: &'a Config,
    files: Vec<String>,
    packages: Vec<(String, String)>,
    root: std::path::PathBuf,
}

impl<'a> Judge<'a> {
    pub fn new(root: &Path, config: &'a Config, merge: &'a Plan) -> Judge<'a> {
        let tree = fairlead_lang::tree::Tree::scan(&fairlead_lang::tree::plain(root));
        let packages = fairlead_lang::workspace::discover(&tree)
            .into_iter()
            .map(|p| (p.name, p.dir))
            .collect();
        Judge {
            merge,
            config,
            files: tree.files,
            packages,
            root: root.to_path_buf(),
        }
    }

    /// The failing test files a runner's output names, each judged; a note
    /// when no `[[replay.failures]]` entry says how to read that runner.
    pub fn judge(&self, inv: &Invocation, log: &str) -> Result<Vec<Verdict>, String> {
        if inv.kind != InvocationKind::Runner {
            return Ok(Vec::new());
        }
        let source = self
            .config
            .replay
            .failures
            .items()
            .iter()
            .find(|f| f.runner == inv.id)
            .ok_or_else(|| {
                format!(
                    "no [[replay.failures]] entry for runner `{}`, so its failing tests can't be named",
                    inv.id
                )
            })?;
        let extractor = Extractor::named(&source.extractor, source.pattern.as_deref())?;
        let repo = Repo {
            files: &self.files,
            packages: &self.packages,
        };
        let read = |f: &str| std::fs::read_to_string(self.root.join(f)).ok();
        let mut tests: Vec<String> = extract(&extractor, log)
            .iter()
            .filter_map(|p| match attribute(p, &repo, read) {
                Attribution::File(f) => Some(f),
                Attribution::Unattributed(_) => None,
            })
            .collect();
        tests.sort();
        tests.dedup();
        Ok(tests
            .into_iter()
            .map(|t| self.verdict(&inv.id, t))
            .collect())
    }

    fn verdict(&self, runner: &str, test: String) -> Verdict {
        let planned = self.merge.all || self.merge.tests.iter().any(|t| t.path == test);
        let fix = (!planned).then(|| match self.merge.changed.first() {
            Some(changed) => format!(
                "[[tests.owners]] match = \"{}\", covers = [\"{}\"]",
                fairlead_replay::report::dir_glob(&test),
                fairlead_replay::report::dir_glob(&changed.path)
            ),
            None => "no rule suggested: the merge changed nothing".into(),
        });
        Verdict {
            runner: runner.to_string(),
            test,
            verdict: if planned { "planned" } else { "escaped" }.into(),
            fix,
        }
    }
}

/// One line per verdict, and a GitHub annotation for each escape when run
/// in Actions.
pub fn print(verdicts: &[Verdict]) {
    let actions = std::env::var("GITHUB_ACTIONS").is_ok_and(|v| v == "true");
    for v in verdicts {
        match &v.fix {
            None => println!(
                "fairlead: {} failed and the merge's plan selected it: it should have failed on the pull request too",
                v.test
            ),
            Some(fix) => {
                println!("fairlead: escape: {} failed and the merge's plan left it out; {fix}", v.test);
                if actions {
                    println!(
                        "::warning file={},title=Fairlead escape::the merge's plan left this test out. {fix}",
                        v.test
                    );
                }
            }
        }
    }
}
