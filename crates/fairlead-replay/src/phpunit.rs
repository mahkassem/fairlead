//! PHPUnit and Pest failures. PHPUnit lists each failure and error as
//! `1) Tests\Unit\FooTest::test_bar` under a `There was 1 failure:` header,
//! followed by the stack, whose frames end in `path.php:line`. Pest, and
//! Laravel's `artisan test`, print `FAILED  Tests\Unit\FooTest > title` and
//! then `at path.php:line`. The frame or `at` path names the test file only
//! when its file name is the class's, since the error may have been thrown
//! in the code under test; otherwise the path comes from the class name,
//! without its first namespace segment, for the suffix match to find.

use std::sync::OnceLock;

use regex::Regex;

use crate::extract::Printed;

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("built-in pattern compiles"))
}

/// How far after a failure's first line its frames may be printed: a
/// PHPUnit message can quote a whole response before them.
const PHPUNIT_LOOKAHEAD: usize = 400;
const PEST_LOOKAHEAD: usize = 60;

/// A frame's `path.php:line`, bare, after `at` or numbered, with Windows
/// separators made forward.
fn frame(line: &str) -> Option<String> {
    static FRAME: OnceLock<Regex> = OnceLock::new();
    let c = re(&FRAME, r"^\s*(?:at\s+|\d+\s+)?(?P<path>\S+\.php):\d+\s*$").captures(line)?;
    Some(c["path"].replace('\\', "/"))
}

/// The test file: a printed path whose file name is the class's, else the
/// class as a path.
fn test_path(class: &str, frames: impl Iterator<Item = String>) -> String {
    // Pest compiles a test file into a class under `P\`.
    let class = class.strip_prefix("P\\").unwrap_or(class);
    let short = class.rsplit('\\').next().unwrap_or(class);
    let file = format!("{short}.php");
    let mut frames = frames;
    if let Some(path) = frames.find(|p| p.rsplit('/').next() == Some(file.as_str())) {
        return path;
    }
    let segments: Vec<&str> = class.split('\\').filter(|s| !s.is_empty()).collect();
    let kept = if segments.len() > 1 {
        &segments[1..]
    } else {
        &segments[..]
    };
    format!("{}.php", kept.join("/"))
}

fn push(out: &mut Vec<Printed>, printed: Printed) {
    if !out.contains(&printed) {
        out.push(printed);
    }
}

pub fn phpunit(lines: &[String]) -> Vec<Printed> {
    static SECTION: OnceLock<Regex> = OnceLock::new();
    static ENTRY: OnceLock<Regex> = OnceLock::new();
    static END: OnceLock<Regex> = OnceLock::new();
    let section = re(
        &SECTION,
        r"^\s*There (?:was|were) \d+ (?P<kind>[\w ]+?):\s*$",
    );
    let entry = re(&ENTRY, r"^\s*\d+\) (?P<class>[\w\\]+)::(?P<method>\w+)");
    let end = re(&END, r"^\s*(?:FAILURES!|ERRORS!|OK \(|Tests: \d+)");
    let mut out = Vec::new();
    let mut counted = false;
    for (i, line) in lines.iter().enumerate() {
        if let Some(c) = section.captures(line) {
            counted = matches!(&c["kind"], "failure" | "failures" | "error" | "errors");
            continue;
        }
        if end.is_match(line) {
            counted = false;
            continue;
        }
        let Some(c) = entry.captures(line).filter(|_| counted) else {
            continue;
        };
        let frames = lines[i + 1..]
            .iter()
            .take(PHPUNIT_LOOKAHEAD)
            .take_while(|l| !entry.is_match(l) && !section.is_match(l) && !end.is_match(l))
            .filter_map(|l| frame(l));
        push(
            &mut out,
            Printed {
                path: test_path(&c["class"], frames),
                project: None,
                title: Some(c["method"].to_string()),
            },
        );
    }
    out
}

pub fn pest(lines: &[String]) -> Vec<Printed> {
    static FAILED: OnceLock<Regex> = OnceLock::new();
    let failed = re(
        &FAILED,
        r"^\s*(?:FAILED|•)\s+(?P<class>[\w\\]+)\s+>\s+(?P<title>.+?)(?:\s{2,}\S+)?\s*$",
    );
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(c) = failed.captures(line) else {
            continue;
        };
        let frames = lines[i + 1..]
            .iter()
            .take(PEST_LOOKAHEAD)
            .take_while(|l| !failed.is_match(l))
            .filter_map(|l| frame(l));
        push(
            &mut out,
            Printed {
                path: test_path(&c["class"], frames),
                project: None,
                title: pest_title(&c["title"]),
            },
        );
    }
    out
}

/// The test's own name, as its source spells it: after the last `describe`
/// arrow, without `it`'s prefix or a dataset suffix. A name cut short to fit
/// the terminal can't be found in the source, so it's dropped.
fn pest_title(printed: &str) -> Option<String> {
    let title = printed.rsplit(" → ").next()?.trim().trim_matches('`');
    let title = title.strip_prefix("it ").unwrap_or(title);
    let title = title.split(" with data set ").next()?;
    let title = title.split(" with dataset ").next()?.trim();
    (!title.is_empty() && !title.contains('…')).then(|| title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn a_frame_in_the_code_under_test_does_not_name_the_test_file() {
        let frames = ["/w/app/app/Models/User.php".to_string()];
        assert_eq!(
            test_path("Tests\\Feature\\UserTest", frames.into_iter()),
            "Feature/UserTest.php"
        );
        let frames = ["/w/app/tests/Feature/UserTest.php".to_string()];
        assert_eq!(
            test_path("Tests\\Feature\\UserTest", frames.into_iter()),
            "/w/app/tests/Feature/UserTest.php"
        );
    }

    #[test]
    fn only_failure_and_error_sections_count() {
        let log = lines(
            "There was 1 risky test:\n\n1) Tests\\Unit\\RiskyTest::test_nothing\nThis test did not perform any assertions\n\nThere were 2 errors:\n\n1) Tests\\Unit\\AlphaTest::test_one\nError: boom\n\nC:\\a\\app\\app\\tests\\Unit\\AlphaTest.php:12\n\n2) Tests\\Unit\\BetaTest::test_two with data set #1 (1, 2)\nTypeError\n\nERRORS!\nTests: 3, Assertions: 1, Errors: 2.\n",
        );
        let found = phpunit(&log);
        assert_eq!(
            found.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(),
            ["C:/a/app/app/tests/Unit/AlphaTest.php", "Unit/BetaTest.php"]
        );
        assert_eq!(found[1].title.as_deref(), Some("test_two"));
    }
}
