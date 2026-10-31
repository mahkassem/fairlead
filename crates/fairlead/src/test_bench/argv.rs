//! The argv each timed run takes, and what its JUnit report says about
//! each file. The runner's own reporter gives way to a JUnit one, so each
//! file's result reads the same whichever mode ran it.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use super::{with_mode, Mode};

fn attr_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r#"\b(file|failures|errors)="([^"]*)""#).expect("valid"))
}

fn suite_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"<testsuite\b([^>]*)>").expect("valid"))
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Each file a JUnit report names, and whether any of its suites failed.
/// Bun names a file per `testsuite` and counts a nested suite's failures in
/// the one above, so the suites alone say it.
pub(super) fn junit_files(report: &str) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for suite in suite_pattern().captures_iter(report) {
        let mut file = None;
        let mut failing = false;
        for a in attr_pattern().captures_iter(&suite[1]) {
            match &a[1] {
                "file" => file = Some(unescape(&a[2])),
                _ => failing |= a[2].trim().parse::<u64>().is_ok_and(|n| n > 0),
            }
        }
        if let Some(file) = file {
            let file = file.replace('\\', "/");
            let file = file.trim_start_matches("./").to_string();
            *out.entry(file).or_insert(false) |= failing;
        }
    }
    out
}

/// A run's failed files: those the report fails, and those it leaves out.
pub(super) fn judge(files: &[String], report: Option<&str>, cwd: &str) -> (usize, Vec<String>) {
    let Some(report) = report else {
        return (0, Vec::new());
    };
    let named = junit_files(report);
    let failed: Vec<String> = files
        .iter()
        .filter(|f| named.get(relative(cwd, f).as_str()) != Some(&false))
        .cloned()
        .collect();
    (files.len() - failed.len(), failed)
}

pub(super) fn relative(cwd: &str, path: &str) -> String {
    if cwd == "." || cwd.is_empty() {
        return path.to_string();
    }
    path.strip_prefix(cwd)
        .and_then(|r| r.strip_prefix('/'))
        .unwrap_or(path)
        .to_string()
}

/// Drops the runner's own reporter, so the JUnit one the bench reads is the only one.
fn without_reporter(command: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for arg in command {
        if std::mem::take(&mut skip_next) {
            continue;
        }
        if arg == "--reporter" || arg == "--reporter-outfile" {
            skip_next = true;
        } else if !(arg.starts_with("--reporter=")
            || arg.starts_with("--reporter-outfile=")
            || arg == "--dots")
        {
            out.push(arg.clone());
        }
    }
    out
}

/// The argv one run of `mode` takes: the runner's command in that mode, a
/// JUnit report to `report`, and the files where `{files}` is or at the end.
pub(super) fn mode_argv(
    command: &[String],
    mode: Mode,
    report: &Path,
    files: &[String],
) -> Vec<String> {
    let mut argv = with_mode(&without_reporter(command), mode);
    let at = 2 + mode.flags().len();
    argv.insert(at.min(argv.len()), "--reporter=junit".into());
    argv.insert(
        (at + 1).min(argv.len()),
        format!("--reporter-outfile={}", report.display()),
    );
    let mut out = Vec::new();
    let mut placed = false;
    for arg in argv {
        if arg == "{files}" {
            out.extend(files.iter().cloned());
            placed = true;
        } else {
            out.push(arg);
        }
    }
    if !placed {
        out.extend(files.iter().cloned());
    }
    out
}

/// Whether a check's command runs `tsc`, as itself or through a package runner.
pub(super) fn runs_tsc(command: &[String]) -> Option<usize> {
    command.iter().position(|a| {
        Path::new(a)
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case("tsc"))
    })
}

/// The `bun check` that stands for a `tsc` command: its project and `--build`.
pub(super) fn bun_check_argv(bun: &str, tsc_args: &[String]) -> Vec<String> {
    let mut out = vec![bun.to_string(), "check".to_string()];
    let mut args = tsc_args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-b" | "--build" => out.push("-b".into()),
            "-p" | "--project" => {
                if let Some(p) = args.next() {
                    out.extend(["-p".to_string(), p.clone()]);
                }
            }
            a if a.starts_with("--project=") => {
                out.extend(["-p".to_string(), a["--project=".len()..].to_string()])
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_junit_report_fails_a_file_by_its_suites_and_a_missing_file_counts_as_failed() {
        let report = r#"<testsuites>
  <testsuite name="a" file="./src/a.test.ts" failures="1">
    <testsuite name="inner" file="./src/a.test.ts" failures="1"></testsuite>
  </testsuite>
  <testsuite name="b" file="src/b&amp;c.test.ts" failures="0" errors="0"></testsuite>
</testsuites>"#;
        let named = junit_files(report);
        assert_eq!(named.get("src/a.test.ts"), Some(&true));
        assert_eq!(named.get("src/b&c.test.ts"), Some(&false));
        let files = argv(&[
            "src/a.test.ts",
            "src/b&c.test.ts",
            "src/never-loaded.test.ts",
        ]);
        let (passed, failed) = judge(&files, Some(report), ".");
        assert_eq!(passed, 1);
        assert_eq!(failed, argv(&["src/a.test.ts", "src/never-loaded.test.ts"]));
        let nested = argv(&["app/src/b&c.test.ts"]);
        assert_eq!(
            judge(&nested, Some(report), "app").0,
            1,
            "paths from the runner's directory"
        );
        assert_eq!(judge(&files, None, "."), (0, Vec::new()));
    }

    #[test]
    fn a_mode_runs_with_a_junit_report_in_place_of_the_runners_own() {
        let command = argv(&[
            "bun",
            "test",
            "--dots",
            "--reporter",
            "junit",
            "--timeout=50",
            "{files}",
        ]);
        let out = mode_argv(
            &command,
            Mode::Parallel,
            Path::new("r.xml"),
            &argv(&["a.test.ts"]),
        );
        assert_eq!(
            out,
            argv(&[
                "bun",
                "test",
                "--parallel",
                "--reporter=junit",
                "--reporter-outfile=r.xml",
                "--timeout=50",
                "a.test.ts"
            ])
        );
        let bare = mode_argv(
            &argv(&["bun", "test"]),
            Mode::Serial,
            Path::new("r.xml"),
            &argv(&["a.test.ts"]),
        );
        assert_eq!(
            bare.last().map(String::as_str),
            Some("a.test.ts"),
            "files go at the end"
        );
    }

    #[test]
    fn bun_check_keeps_the_tsc_project_and_build_flags() {
        assert_eq!(runs_tsc(&argv(&["bunx", "tsc", "-b"])), Some(1));
        assert_eq!(runs_tsc(&argv(&["node_modules/.bin/tsc"])), Some(0));
        assert_eq!(runs_tsc(&argv(&["eslint", "."])), None);
        assert_eq!(
            bun_check_argv(
                "bun",
                &argv(&["-b", "--noEmit", "--project", "app/tsconfig.json"])
            ),
            argv(&["bun", "check", "-b", "-p", "app/tsconfig.json"])
        );
        assert_eq!(
            bun_check_argv("bun", &argv(&["--project=x.json"])),
            argv(&["bun", "check", "-p", "x.json"])
        );
    }
}
