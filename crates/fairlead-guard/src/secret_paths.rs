//! `[guard.paths]`, rule `secret-path`: files whose name says they hold a
//! secret, such as a keystore or an `.env` file. It reads paths, never
//! contents, so a binary keystore is seen as well as a text one. A commit
//! may not add one; the check stage reports the ones already tracked.

use fairlead_core::config;
use fairlead_core::pattern::Pattern;

use crate::finding::Finding;
use crate::git::Staged;
use crate::rules::compile;

pub const RULE: &str = "secret-path";

pub struct SecretPaths {
    deny: Vec<Pattern>,
    allow: Vec<Pattern>,
    fixtures: Vec<String>,
    /// None: tracked matches are warnings; else as other presets' `ratchet`.
    pub ratchet: Option<bool>,
}

/// What a stage found: findings that count, and warnings that never fail.
#[derive(Debug, Default)]
pub struct Found {
    pub findings: Vec<Finding>,
    pub warnings: Vec<Finding>,
}

fn finding(path: &str, message: String) -> Finding {
    Finding {
        file: path.to_string(),
        line: 1,
        rule: RULE,
        message,
        anchor: String::new(),
        measure: None,
    }
}

impl SecretPaths {
    pub fn new(c: &config::SecretPaths) -> Result<SecretPaths, String> {
        Ok(SecretPaths {
            deny: compile(c.deny.items())?,
            allow: compile(c.allow.items())?,
            fixtures: c.fixtures.items().to_vec(),
            ratchet: c.ratchet,
        })
    }

    /// The deny glob a path matches, unless an allow glob lets it through.
    pub fn matched(&self, path: &str) -> Option<&str> {
        if self.allow.iter().any(|p| p.is_match(path)) {
            return None;
        }
        self.deny
            .iter()
            .find(|p| p.is_match(path))
            .map(Pattern::source)
    }

    /// Under a directory named as a fixture, such as `tests/`, where a key
    /// is usually made for the test.
    fn fixture(&self, path: &str) -> bool {
        let mut dirs = path.split('/').rev().skip(1);
        dirs.any(|d| self.fixtures.iter().any(|f| f == d))
    }

    /// Each staged file the commit newly adds, or renames into a match from
    /// a name that didn't match. A fixture's is a warning.
    pub fn staged(&self, staged: &[Staged]) -> Found {
        let mut found = Found::default();
        for s in staged {
            let new = match s.status {
                'A' | 'C' => true,
                'R' => s
                    .before
                    .as_deref()
                    .is_none_or(|b| self.matched(b).is_none()),
                _ => false,
            };
            let Some(glob) = self.matched(&s.path).filter(|_| new) else {
                continue;
            };
            let message = format!(
                "adds a file named like a secret (`{glob}`); keep it out of git (add it to .gitignore and `git rm --cached` it), or list it under guard.paths.allow"
            );
            if self.fixture(&s.path) {
                found.warnings.push(finding(&s.path, message));
            } else {
                found.findings.push(finding(&s.path, message));
            }
        }
        found
    }

    /// Each tracked match. Until `ratchet` is set they're warnings, so an
    /// upgrade never fails on a file history already holds.
    pub fn tracked(&self, tracked: &[String]) -> Found {
        let mut found = Found::default();
        for path in tracked {
            let Some(glob) = self.matched(path) else {
                continue;
            };
            let f = finding(
                path,
                format!("is tracked and named like a secret (`{glob}`); if it holds one, rotate it and remove the file, or list it under guard.paths.allow"),
            );
            if self.ratchet.is_none() || self.fixture(path) {
                found.warnings.push(f);
            } else {
                found.findings.push(f);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built_in() -> SecretPaths {
        SecretPaths::new(&config::SecretPaths::default()).unwrap()
    }

    fn staged(status: char, before: Option<&str>, path: &str) -> Staged {
        Staged {
            status,
            before: before.map(String::from),
            path: path.into(),
        }
    }

    #[test]
    fn the_built_in_lists_catch_keystores_and_env_files_but_not_their_templates() {
        let p = built_in();
        for hit in [
            "android/app/release.jks",
            ".env",
            "api/.env.production",
            "certs/server.pem",
            "deploy/key.txt",
            "client_secret_123.json",
            "ios/App.mobileprovision",
            "a/b.p12",
        ] {
            assert!(p.matched(hit).is_some(), "{hit}");
        }
        for miss in [
            ".env.example",
            "web/.env.sample",
            ".env.test",
            ".env.ci",
            "id_ed25519.pub",
            "config/key.txt.example",
            "src/environment.ts",
            "docs/keys.md",
        ] {
            assert!(p.matched(miss).is_none(), "{miss}");
        }
    }

    #[test]
    fn a_commit_fails_only_on_a_file_it_adds_or_renames_into_a_match() {
        let p = built_in();
        let found = p.staged(&[
            staged('A', None, "app/upload.jks"),
            staged('M', Some(".env"), ".env"),
            staged('R', Some("notes.txt"), "secret/.env.local"),
            staged('R', Some("old/.env"), "new/.env"),
            staged('A', None, "README.md"),
        ]);
        let files: Vec<&str> = found.findings.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(files, ["app/upload.jks", "secret/.env.local"]);
        assert!(found.warnings.is_empty());
    }

    #[test]
    fn a_key_under_a_fixture_directory_is_a_warning() {
        let p = built_in();
        let found = p.staged(&[staged('A', None, "tests/fixtures/server.key")]);
        assert!(found.findings.is_empty());
        assert_eq!(found.warnings.len(), 1);
        assert_eq!(p.tracked(&["spec/a.pem".into()]).warnings.len(), 1);
    }

    #[test]
    fn tracked_matches_are_warnings_until_ratchet_is_set() {
        let tracked = vec![".env.production".to_string(), "src/a.ts".to_string()];
        let found = built_in().tracked(&tracked);
        assert_eq!((found.findings.len(), found.warnings.len()), (0, 1));
        let strict = SecretPaths::new(&config::SecretPaths {
            ratchet: Some(false),
            ..config::SecretPaths::default()
        })
        .unwrap();
        assert_eq!(strict.tracked(&tracked).findings.len(), 1);
    }
}
