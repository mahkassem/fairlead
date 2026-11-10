//! Which kind of project a repository is, for `fairlead setup`: new, mvp,
//! ongoing or legacy, read from signals that are cheap and local. The track
//! decides which setup items are required and the order they print in; it
//! is detected on every run and never written to the config.

use std::path::Path;

use fairlead_core::config::Config;
use fairlead_core::pattern::Pattern;
use fairlead_lang::tree::Tree;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Track {
    /// No tests yet and little history: the guard first.
    New,
    /// Tests, and nothing that runs them or deploys on its own yet.
    Mvp,
    /// A workflow tests or deploys it, and it has many tests or a long history.
    Ongoing,
    /// A long history the planner reads little of.
    Legacy,
}

impl Track {
    pub fn name(self) -> &'static str {
        match self {
            Track::New => "new",
            Track::Mvp => "mvp",
            Track::Ongoing => "ongoing",
            Track::Legacy => "legacy",
        }
    }
}

/// The track a repository is on, and the numbers that chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub track: Track,
    pub why: String,
}

/// Counting stops here: past it a repository isn't new, whatever the number.
const COMMIT_CAP: usize = 1000;
const NEW_COMMITS: usize = 20;
const OLD_DAYS: i64 = 180;
const MANY_TESTS: usize = 50;
/// A long history with fewer tests than one per this many source files is legacy.
const SOURCES_PER_TEST: usize = 20;

/// How test files are named where no config says: each language's habit.
const STANDARD_TESTS: [&str; 11] = [
    "**/*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}",
    "**/*_test.go",
    "**/test_*.py",
    "**/*_test.py",
    "**/*Test.php",
    "**/*_test.rb",
    "**/*_spec.rb",
    "**/*Test.java",
    "**/*Test.kt",
    "**/*Tests.cs",
    "**/*_test.dart",
];

/// A file that says which stack this is, by its name.
const MANIFESTS: [&str; 14] = [
    "package.json",
    "go.mod",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "composer.json",
    "Cargo.toml",
    "Gemfile",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "mix.exs",
    "pubspec.yaml",
    "Package.swift",
];

/// Text in a workflow that runs a test suite. A hint only, as `phase`'s
/// deploy markers are.
const TEST_COMMANDS: [&str; 20] = [
    "fairlead ci",
    "npm test",
    "npm run test",
    "pnpm test",
    "pnpm run test",
    "yarn test",
    "bun test",
    "bun run test",
    "vitest",
    "jest",
    "playwright test",
    "pytest",
    "go test",
    "cargo test",
    "phpunit",
    "artisan test",
    "rspec",
    "gradlew test",
    "mvn test",
    "dotnet test",
];

/// What detection counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Signals {
    tests: usize,
    /// Source files the graph reads, tests left out.
    sources: usize,
    /// Capped at `COMMIT_CAP`.
    commits: usize,
    /// The first commit's age.
    days: i64,
    /// Workflows that run tests or deploy.
    workflows: usize,
    /// A manifest names a stack and `init` finds no runner for it.
    unread: bool,
    /// How many files are named like a secret, such as a keystore or an
    /// `.env` file: a count, never a name or a value.
    flagged: usize,
}

pub fn detect(root: &Path, files: &[String], config: Option<&Config>) -> Detected {
    decide(&signals(root, files, config))
}

fn signals(root: &Path, files: &[String], config: Option<&Config>) -> Signals {
    let planned = config.filter(|c| c.profile.is_full());
    let own = |f: &&String| !f.starts_with("vendor/") && !f.contains("/vendor/");
    let tests = test_patterns(planned);
    let is_test =
        |f: &str| tests.0.iter().any(|p| p.is_match(f)) && !tests.1.iter().any(|p| p.is_match(f));
    let test_files = files.iter().filter(own).filter(|f| is_test(f)).count();
    let sources = files
        .iter()
        .filter(own)
        .filter(|f| Tree::is_source(f) && !is_test(f))
        .count();
    let manifest = files.iter().filter(own).any(|f| {
        let name = f.rsplit('/').next().unwrap_or(f);
        MANIFESTS.contains(&name) || name.ends_with(".csproj") || name.ends_with(".sln")
    });
    let configured = planned.is_some_and(|c| !c.tests.runners.items().is_empty());
    let unread = manifest && !configured && crate::init_cmd::runners_in(root, files).is_empty();
    let found = crate::phase::workflows(root);
    let workflows = found
        .iter()
        .filter(|w| {
            runs_tests(&w.text) || !crate::phase::live_evidence(std::slice::from_ref(w)).is_empty()
        })
        .count();
    let flagged = fairlead_guard::secret_paths::SecretPaths::new(&Default::default())
        .map_or(0, |s| {
            files.iter().filter(|f| s.matched(f).is_some()).count()
        });
    let (commits, days) = history(root);
    Signals {
        tests: test_files,
        sources,
        commits,
        days,
        workflows,
        unread,
        flagged,
    }
}

/// The patterns that name test files and the ones that leave a file out:
/// the config's where it plans tests, else each language's habit.
fn test_patterns(planned: Option<&Config>) -> (Vec<Pattern>, Vec<Pattern>) {
    let compile = |globs: &[String]| -> Vec<Pattern> {
        globs.iter().filter_map(|g| Pattern::new(g).ok()).collect()
    };
    match planned {
        Some(c) => (
            compile(c.tests.matches.items()),
            compile(c.tests.exclude.items()),
        ),
        None => (
            STANDARD_TESTS
                .iter()
                .filter_map(|g| Pattern::new(g).ok())
                .collect(),
            Vec::new(),
        ),
    }
}

fn runs_tests(workflow: &str) -> bool {
    workflow
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .any(|l| TEST_COMMANDS.iter().any(|c| l.contains(c)))
}

/// The commits on HEAD, capped, and the age in days of the first one; both
/// 0 where git has no history to show.
fn history(root: &Path) -> (usize, i64) {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let cap = format!("--max-count={COMMIT_CAP}");
    let commits = git(&["rev-list", "--count", &cap, "HEAD"])
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0);
    let first = git(&["log", "--max-parents=0", "--format=%ct", "HEAD"])
        .and_then(|t| t.lines().filter_map(|l| l.trim().parse::<i64>().ok()).min());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let days = first.map_or(0, |t| ((now - t) / 86_400).max(0));
    (commits, days)
}

fn plural(n: usize, one: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    }
}

/// A history's length in the largest unit that reads as a whole number.
fn age(days: i64) -> String {
    let n = |count: i64, unit: &str| plural(count as usize, unit);
    match days {
        d if d < 1 => "under a day".to_string(),
        d if d < 60 => n(d, "day"),
        d if d < 730 => n(d / 30, "month"),
        d => n(d / 365, "year"),
    }
}

/// The rules, first match wins.
fn decide(s: &Signals) -> Detected {
    let commits = match s.commits {
        n if n >= COMMIT_CAP => format!("{COMMIT_CAP}+ commits"),
        n => plural(n, "commit"),
    };
    let history = format!("{} of history", age(s.days));
    let tests = plural(s.tests, "test file");
    let workflows = match s.workflows {
        0 => "no workflow that tests or deploys".to_string(),
        n => plural(n, "workflow"),
    };
    let old = s.days > OLD_DAYS;
    let thin = s.tests * SOURCES_PER_TEST < s.sources;
    let (track, why) = if s.tests == 0 && s.commits < NEW_COMMITS {
        (Track::New, format!("no tests yet, {commits}"))
    } else if old && (s.unread || thin) {
        let reads = match s.unread {
            true => "the planner can't read the stack".to_string(),
            false => format!("{tests} for {}", plural(s.sources, "source file")),
        };
        let flagged = match s.flagged {
            0 => String::new(),
            n => format!(", {} named like a secret", plural(n, "file")),
        };
        (Track::Legacy, format!("{history}, {reads}{flagged}"))
    } else if s.workflows > 0 && (s.tests > MANY_TESTS || old) {
        (Track::Ongoing, format!("{tests}, {workflows}, {history}"))
    } else {
        (Track::Mvp, format!("{tests}, {workflows}, {history}"))
    };
    Detected { track, why }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted(tests: usize, sources: usize, commits: usize, days: i64) -> Signals {
        Signals {
            tests,
            sources,
            commits,
            days,
            ..Signals::default()
        }
    }

    #[test]
    fn no_tests_and_under_20_commits_is_new_and_20_commits_is_not() {
        let found = decide(&counted(0, 40, 19, 3));
        assert_eq!(found.track, Track::New);
        assert_eq!(found.why, "no tests yet, 19 commits");
        assert_eq!(decide(&counted(0, 40, 20, 3)).track, Track::Mvp);
        assert_eq!(decide(&counted(1, 40, 2, 3)).track, Track::Mvp);
    }

    #[test]
    fn a_long_history_the_planner_cannot_read_is_legacy() {
        let found = decide(&Signals {
            unread: true,
            workflows: 2,
            flagged: 3,
            ..counted(30, 100, 500, 800)
        });
        assert_eq!(found.track, Track::Legacy);
        assert_eq!(
            found.why,
            "2 years of history, the planner can't read the stack, 3 files named like a secret"
        );
        // The same repository at 180 days isn't old yet.
        let young = decide(&Signals {
            unread: true,
            ..counted(30, 100, 500, 180)
        });
        assert_eq!(young.track, Track::Mvp);
    }

    #[test]
    fn a_long_history_with_under_one_test_per_20_sources_is_legacy() {
        let found = decide(&counted(4, 81, 300, 181));
        assert_eq!(found.track, Track::Legacy);
        assert_eq!(
            found.why,
            "6 months of history, 4 test files for 81 source files"
        );
        // One test per 20 sources is enough to plan with.
        assert_eq!(decide(&counted(4, 80, 300, 181)).track, Track::Mvp);
    }

    #[test]
    fn a_workflow_with_many_tests_or_a_long_history_is_ongoing() {
        let many = decide(&Signals {
            workflows: 3,
            ..counted(412, 900, COMMIT_CAP, 900)
        });
        assert_eq!(many.track, Track::Ongoing);
        assert_eq!(many.why, "412 test files, 3 workflows, 2 years of history");
        let young = Signals {
            workflows: 1,
            ..counted(51, 100, 60, 20)
        };
        assert_eq!(decide(&young).track, Track::Ongoing);
        let old = Signals {
            workflows: 1,
            ..counted(10, 100, 60, 181)
        };
        assert_eq!(decide(&old).track, Track::Ongoing);
        // Without the workflow, or with neither size nor age, it's an mvp.
        let unrun = Signals {
            workflows: 0,
            ..young.clone()
        };
        assert_eq!(decide(&unrun).track, Track::Mvp);
        let small = Signals { tests: 50, ..young };
        assert_eq!(decide(&small).track, Track::Mvp);
    }

    #[test]
    fn everything_else_is_an_mvp_and_says_what_it_counted() {
        let found = decide(&counted(3, 12, 45, 1));
        assert_eq!(found.track, Track::Mvp);
        assert_eq!(
            found.why,
            "3 test files, no workflow that tests or deploys, 1 day of history"
        );
        assert_eq!(age(0), "under a day");
        assert_eq!(age(59), "59 days");
        assert_eq!(age(60), "2 months");
        assert_eq!(age(729), "24 months");
        assert_eq!(age(730), "2 years");
    }

    #[test]
    fn signals_count_tests_by_habit_without_a_config_and_by_the_config_with_one() {
        let dir = std::env::temp_dir().join(format!("fl-track-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        std::fs::write(
            dir.join(".github/workflows/ci.yml"),
            "jobs:\n  t:\n    steps:\n      - run: go test ./...\n",
        )
        .unwrap();
        std::fs::write(
            dir.join(".github/workflows/lint.yml"),
            "jobs:\n  l:\n    steps:\n      # go test ./...\n      - run: golangci-lint run\n",
        )
        .unwrap();
        let files: Vec<String> = [
            "go.mod",
            "cart/cart.go",
            "cart/cart_test.go",
            "web/a.ts",
            "web/a.test.ts",
            "vendor/x/x_test.go",
            ".env",
        ]
        .map(String::from)
        .to_vec();
        let found = signals(&dir, &files, None);
        assert_eq!((found.tests, found.sources), (2, 2), "{found:?}");
        assert_eq!((found.workflows, found.flagged), (1, 1), "{found:?}");
        assert!(!found.unread, "go.mod and a Go test: init has a runner");
        let config: Config = toml::from_str(
            "[tests]\nmatch = { replace = [\"**/*_test.go\"] }\n[[tests.runners]]\nid = \"go\"\nmatch = [\"**/*_test.go\"]\ncommand = [\"go\", \"test\", \"{packages}\"]\n",
        )
        .unwrap();
        let found = signals(&dir, &files, Some(&config));
        assert_eq!((found.tests, found.sources), (1, 3), "{found:?}");
        // A manifest whose stack init finds no runner for is one the planner can't read.
        let rust = ["Cargo.toml", "src/lib.rs"].map(String::from).to_vec();
        assert!(signals(&dir, &rust, None).unread);
        assert!(!signals(&dir, &["README.md".to_string()], None).unread);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
