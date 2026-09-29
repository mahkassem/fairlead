//! Matching a printed path to a file in the repository at the failing
//! commit. Runners print paths relative to a project or package, so the
//! match tries, in order: the path as it stands, the path under the named
//! project (a workspace package's name or folder), a unique suffix, then
//! the suffix candidates whose source contains the failing test's title.
//! Anything still ambiguous is unattributed, never guessed.

use crate::extract::Printed;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attribution {
    File(String),
    /// No file matched, or several did; the candidates, if any.
    Unattributed(Vec<String>),
}

pub struct Repo<'a> {
    pub files: &'a [String],
    /// Workspace packages as (name, folder).
    pub packages: &'a [(String, String)],
}

fn under(dir: &str, path: &str) -> String {
    if dir.is_empty() {
        path.to_string()
    } else {
        format!("{dir}/{path}")
    }
}

/// `/home/runner/work/repo/repo/x` or `D:/a/repo/repo/x`: a path on the
/// runner, not relative to anything the repository knows.
fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/')
        || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/')
}

/// The longest tail of a runner's path, at a directory boundary, that is a
/// file in the repository: the checkout's own directories come first. A
/// Go import path's tail may sit below a module's folder, so failing that,
/// the longest tail of two segments or more that ends exactly one file.
fn absolute(path: &str, files: &[String], has: impl Fn(&str) -> bool) -> Attribution {
    let tails: Vec<&str> = path
        .char_indices()
        .filter(|&(_, c)| c == '/')
        .map(|(i, _)| &path[i + 1..])
        .filter(|t| !t.is_empty())
        .collect();
    if let Some(tail) = tails.iter().find(|t| has(t)) {
        return Attribution::File(tail.to_string());
    }
    for tail in tails.iter().filter(|t| t.contains('/')) {
        let suffix = format!("/{tail}");
        let found: Vec<&String> = files.iter().filter(|f| f.ends_with(&suffix)).collect();
        match found.as_slice() {
            [one] => return Attribution::File((*one).clone()),
            [] => continue,
            many => return Attribution::Unattributed(many.iter().map(|f| f.to_string()).collect()),
        }
    }
    Attribution::Unattributed(Vec::new())
}

pub fn attribute(
    printed: &Printed,
    repo: &Repo,
    read: impl Fn(&str) -> Option<String>,
) -> Attribution {
    // A Windows runner prints `tests\unit\a.py`; no repository path has a backslash.
    let normalized = printed.path.replace('\\', "/");
    let path = normalized.trim_start_matches("./");
    let has = |p: &str| repo.files.iter().any(|f| f == p);
    if has(path) {
        return Attribution::File(path.to_string());
    }
    if is_absolute(path) {
        return absolute(path, repo.files, has);
    }
    if let Some(project) = &printed.project {
        let dir = repo
            .packages
            .iter()
            .find(|(name, dir)| name == project || dir == project)
            .map(|(_, dir)| dir.clone())
            .or_else(|| {
                repo.files
                    .iter()
                    .any(|f| f.starts_with(&format!("{project}/")))
                    .then(|| project.clone())
            });
        if let Some(dir) = dir {
            let joined = under(&dir, path);
            if has(&joined) {
                return Attribution::File(joined);
            }
        }
    }
    let suffix = format!("/{path}");
    let candidates: Vec<String> = repo
        .files
        .iter()
        .filter(|f| f.ends_with(&suffix))
        .cloned()
        .collect();
    if candidates.len() == 1 {
        return Attribution::File(candidates[0].clone());
    }
    if let Some(title) = printed.title.as_deref().filter(|t| t.len() >= 8) {
        let named: Vec<&String> = candidates
            .iter()
            .filter(|f| read(f).is_some_and(|text| text.contains(title)))
            .collect();
        if named.len() == 1 {
            return Attribution::File(named[0].clone());
        }
    }
    Attribution::Unattributed(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printed(path: &str, project: Option<&str>, title: Option<&str>) -> Printed {
        Printed {
            path: path.into(),
            project: project.map(Into::into),
            title: title.map(Into::into),
        }
    }

    fn files() -> Vec<String> {
        [
            "packages/effect/test/Pool.test.ts",
            "packages/sql/test/Pool.test.ts",
            "pkg/a/test/index.ts",
            "pkg/b/test/index.ts",
            "specs/bail-out.test.ts",
            "tests/Unit/Billing/PricingTest.php",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn a_project_name_picks_its_package_folder() {
        let files = files();
        let packages = vec![("effect".to_string(), "packages/effect".to_string())];
        let repo = Repo {
            files: &files,
            packages: &packages,
        };
        let found = attribute(
            &printed("test/Pool.test.ts", Some("effect"), None),
            &repo,
            |_| None,
        );
        assert_eq!(
            found,
            Attribution::File("packages/effect/test/Pool.test.ts".into())
        );
    }

    #[test]
    fn a_unique_suffix_matches_and_an_ambiguous_one_needs_the_title() {
        let files = files();
        let repo = Repo {
            files: &files,
            packages: &[],
        };
        assert_eq!(
            attribute(&printed("bail-out.test.ts", None, None), &repo, |_| None),
            Attribution::File("specs/bail-out.test.ts".into())
        );
        let ambiguous = printed("test/index.ts", None, Some("exit with error on INT signal"));
        assert!(
            matches!(attribute(&ambiguous, &repo, |_| None), Attribution::Unattributed(c) if c.len() == 2)
        );
        let read = |f: &str| {
            (f == "pkg/b/test/index.ts")
                .then(|| "test('exit with error on INT signal', ...)".to_string())
        };
        assert_eq!(
            attribute(&ambiguous, &repo, read),
            Attribution::File("pkg/b/test/index.ts".into())
        );
    }

    #[test]
    fn a_runners_absolute_path_matches_its_longest_tail_in_the_repository() {
        let files = files();
        let repo = Repo {
            files: &files,
            packages: &[],
        };
        let at = |p: &str| attribute(&printed(p, None, None), &repo, |_| None);
        let want = Attribution::File("tests/Unit/Billing/PricingTest.php".into());
        assert_eq!(
            at("/home/runner/work/shop/shop/tests/Unit/Billing/PricingTest.php"),
            want
        );
        assert_eq!(
            at("D:/a/shop/shop/tests/Unit/Billing/PricingTest.php"),
            want
        );
        assert_eq!(
            at("/home/runner/work/shop/shop/vendor/x/Y.php"),
            Attribution::Unattributed(Vec::new())
        );
        assert_eq!(
            at("/example.com/shop/Billing/PricingTest.php"),
            want,
            "an import path below a module's folder, by its unique suffix"
        );
    }

    #[test]
    fn a_windows_runners_backslashes_match_the_repository_path() {
        let files = files();
        let repo = Repo {
            files: &files,
            packages: &[],
        };
        let want = Attribution::File("tests/Unit/Billing/PricingTest.php".into());
        assert_eq!(
            attribute(
                &printed("tests\\Unit\\Billing\\PricingTest.php", None, None),
                &repo,
                |_| None
            ),
            want
        );
        assert_eq!(
            attribute(
                &printed("Unit\\Billing\\PricingTest.php", None, None),
                &repo,
                |_| None
            ),
            want,
            "and by suffix"
        );
    }
}
