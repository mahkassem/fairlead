//! Maven (Surefire and Failsafe) and Gradle failures. Both name a failing
//! test by its class, never its file. Surefire prints a class's
//! `[ERROR] Tests run: … <<< FAILURE! -- in com.acme.FooTest`, then each
//! failed test as `[ERROR] com.acme.FooTest.method -- Time elapsed: … <<<
//! FAILURE!` (`method(com.acme.FooTest)` before 3.0), and repeats them in
//! its closing `[ERROR] Failures:` and `Errors:` lists by simple name.
//! Gradle prints `FooTest > method() FAILED`, by simple name under JUnit 5
//! and qualified under JUnit 4, under the `> Task :module:test` header its
//! grouped output repeats whenever the task printing changes. The class
//! becomes a path, `com/acme/FooTest.java`, that attribution matches to a
//! Java or Kotlin file; a nested class stands for its outermost class's.

use std::sync::OnceLock;

use regex::Regex;

use crate::extract::Printed;

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("built-in pattern compiles"))
}

fn push(out: &mut Vec<Printed>, printed: Printed) {
    if !out.contains(&printed) {
        out.push(printed);
    }
}

fn identifier(segment: &str, nested: bool) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || (nested && c == '$'))
}

/// A class name as a path, and the member printed after it, if any. The
/// class is the first segment that starts with a capital, as Java and
/// Kotlin name classes and not packages; a nested class, `Outer$Inner` or
/// `Outer.Inner`, is its outermost class's file. Parameters and a
/// `[index]` or `[target]` suffix are dropped.
pub(crate) fn class_path(name: &str) -> Option<(String, Option<String>)> {
    let head = name.split(['(', '[']).next()?.trim();
    let segments: Vec<&str> = head.split('.').collect();
    let at = segments
        .iter()
        .position(|s| s.starts_with(|c: char| c.is_uppercase()))?;
    if !segments[..at].iter().all(|s| identifier(s, false)) || !identifier(segments[at], true) {
        return None;
    }
    let outer = segments[at].split('$').next()?;
    let mut path: Vec<&str> = segments[..at].to_vec();
    path.push(outer);
    let member = segments[at + 1..]
        .last()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    Some((format!("{}.java", path.join("/")), member))
}

fn printed(path: String, project: Option<String>, title: Option<String>) -> Printed {
    Printed {
        path,
        project,
        title,
    }
}

/// The class file's simple name, `FooTest` for `com/acme/FooTest.java`.
fn simple(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(".java").unwrap_or(file)
}

/// A failed test's name: `com.acme.FooTest.method` since Surefire 3,
/// `method(com.acme.FooTest)` before it, or the class alone when the class
/// itself failed, such as in a `@BeforeAll`.
fn surefire_test(name: &str) -> Option<(String, Option<String>)> {
    static OLD: OnceLock<Regex> = OnceLock::new();
    let old = re(
        &OLD,
        r"^(?P<method>[^\s().]+?)(?:\[[^\]]*\])?\((?P<class>[\w.$]+)\)$",
    );
    match old.captures(name) {
        Some(c) => class_path(&c["class"]).map(|(path, _)| (path, Some(c["method"].to_string()))),
        None => class_path(name),
    }
}

/// Whether a line still belongs to Surefire's closing `Failures:` or
/// `Errors:` list: its entries, and the blank line after a re-run's runs.
pub(crate) fn in_surefire_list(line: &str) -> bool {
    let line = line.trim_end();
    (line.starts_with("[ERROR]") && !line.contains("Tests run:")) || line == "[INFO]"
}

pub fn maven(lines: &[String]) -> Vec<Printed> {
    static CLASS: OnceLock<Regex> = OnceLock::new();
    static TEST: OnceLock<Regex> = OnceLock::new();
    static LIST: OnceLock<Regex> = OnceLock::new();
    static ENTRY: OnceLock<Regex> = OnceLock::new();
    let class = re(
        &CLASS,
        r"^\[ERROR\] Tests run: .*<<< (?:FAILURE|ERROR)!\s+--?\s+in\s+(?P<class>[\w.$]+)",
    );
    let test = re(
        &TEST,
        r"^\[ERROR\] (?P<name>.+?)\s+(?:--\s+)?Time elapsed: .*<<< (?:FAILURE|ERROR)!\s*$",
    );
    let list = re(&LIST, r"^\[ERROR\] (?:Failures|Errors):\s*$");
    // `FooTest.method:42 message`, `FooTest>Base.method:9`, a re-run's
    // `com.acme.FooTest.method` and its `Run 1: FooTest.method:42`.
    let entry = re(
        &ENTRY,
        r"^\[ERROR\]\s+(?:Run \d+: )?(?P<class>[\w$]+(?:\.[\w$]+)*?)(?:>[\w$.]+)?\.(?P<method>[\w$]+)(?::\d+|\s|$)",
    );
    let mut out = Vec::new();
    // Classes in the order their summary line failed, and whether one of
    // their tests was named.
    let mut classes: Vec<(String, bool)> = Vec::new();
    let mut listed: Vec<Printed> = Vec::new();
    let mut in_list = false;
    for line in lines {
        if list.is_match(line) {
            in_list = true;
            continue;
        }
        // An excerpt can drop the list's closing `Tests run:` line, so a
        // class's or a test's own line ends it too.
        if in_list && (!in_surefire_list(line) || class.is_match(line) || test.is_match(line)) {
            in_list = false;
        }
        if in_list {
            if let Some((path, method)) = entry
                .captures(line)
                .and_then(|c| class_path(&c["class"]).map(|(p, _)| (p, c["method"].to_string())))
            {
                listed.push(printed(path, None, Some(method)));
            }
            continue;
        }
        if let Some(c) = class.captures(line) {
            if let Some((path, _)) = class_path(&c["class"]) {
                if !classes.iter().any(|(p, _)| *p == path) {
                    classes.push((path, false));
                }
            }
            continue;
        }
        let Some((path, method)) = test.captures(line).and_then(|c| surefire_test(&c["name"]))
        else {
            continue;
        };
        match classes.iter_mut().find(|(p, _)| *p == path) {
            Some(seen) => seen.1 = true,
            None => classes.push((path.clone(), true)),
        }
        push(&mut out, printed(path, None, method));
    }
    for (path, named) in &classes {
        if !named {
            push(&mut out, printed(path.clone(), None, None));
        }
    }
    // The closing lists repeat what was printed above by simple name, so
    // they count only for a class nothing above named.
    for p in listed {
        if !classes.iter().any(|(c, _)| simple(c) == simple(&p.path)) {
            push(&mut out, p);
        }
    }
    out
}

/// A Gradle task's project as a folder: `:core:jvm:test` is `core/jvm`.
fn project_dir(task: &str) -> Option<String> {
    let (project, _) = task.rsplit_once(':')?;
    let dir = project.trim_start_matches(':').replace(':', "/");
    (!dir.is_empty()).then_some(dir)
}

/// The test's own name in `Class > … > name FAILED`: the last part that is
/// a method, `name()` or `name(String)`, else the last part, so a nested
/// class's test and a parameterized one's `[1] …` case both name the method.
fn gradle_title(parts: &[&str]) -> Option<String> {
    let strip = |p: &str| {
        let p = p.trim();
        let p = p
            .strip_suffix(']')
            .map_or(p, |q| q.rsplit_once('[').map_or(q, |(a, _)| a));
        p.trim().to_string()
    };
    let method = parts[1..]
        .iter()
        .rev()
        .map(|p| strip(p))
        .find(|p| p.ends_with(')'));
    let title = method
        .map(|m| m.split('(').next().unwrap_or(&m).trim().to_string())
        .or_else(|| parts.last().map(|p| strip(p)))?;
    (!title.is_empty()).then_some(title)
}

/// A path a tool printed as a file URL or a plain path, with Windows'
/// `/D:/...` form put back to `D:/...`.
fn local(path: &str) -> String {
    let path = path.strip_prefix("file://").unwrap_or(path);
    match path.strip_prefix('/') {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => path.to_string(),
    }
}

/// The source file of a Kotlin or Java compile error, while a task that
/// compiles tests runs: a test file that doesn't compile is that file's failure.
fn test_compile_error(task: &str, line: &str) -> Option<String> {
    static KOTLIN: OnceLock<Regex> = OnceLock::new();
    static JAVA: OnceLock<Regex> = OnceLock::new();
    let name = task.rsplit(':').next().unwrap_or(task).to_ascii_lowercase();
    if !(name.starts_with("compile") && name.contains("test")) {
        return None;
    }
    let kotlin = re(&KOTLIN, r"^e: (?P<path>\S+?\.kts?):\d+:\d+");
    let java = re(&JAVA, r"^(?P<path>\S+?\.java):\d+: error:");
    let c = kotlin.captures(line).or_else(|| java.captures(line))?;
    Some(local(&c["path"]))
}

pub fn gradle(lines: &[String]) -> Vec<Printed> {
    static TASK: OnceLock<Regex> = OnceLock::new();
    static FAILED: OnceLock<Regex> = OnceLock::new();
    static EXECUTION: OnceLock<Regex> = OnceLock::new();
    static REPORT: OnceLock<Regex> = OnceLock::new();
    let task = re(&TASK, r"^> Task (?P<task>:\S+)");
    let failed = re(&FAILED, r"^(?P<chain>[^\s>].*? > .+?) FAILED\s*$");
    let execution = re(&EXECUTION, r"Execution failed for task '(?P<task>:[^']+)'");
    let report = re(
        &REPORT,
        r"There were failing tests\. See the report at: (?:file://)?(?P<path>\S+)",
    );
    let mut out = Vec::new();
    let mut project: Option<String> = None;
    let mut current = String::new();
    let mut failing: Option<Option<String>> = None;
    let mut named: Vec<Option<String>> = Vec::new();
    for line in lines {
        if let Some(c) = task.captures(line) {
            project = project_dir(&c["task"]);
            current = c["task"].to_string();
            continue;
        }
        if let Some(path) = test_compile_error(&current, line) {
            if !named.contains(&project) {
                named.push(project.clone());
            }
            push(&mut out, printed(path, project.clone(), None));
            continue;
        }
        if let Some(c) = execution.captures(line) {
            failing = Some(project_dir(&c["task"]));
            continue;
        }
        // The closing section names the task and its report, not a test:
        // a task none of whose tests was named is a failure left unattributed.
        if let Some(c) = report.captures(line) {
            let task_project = failing.take().unwrap_or_else(|| project.clone());
            if !named.contains(&task_project) {
                push(&mut out, printed(local(&c["path"]), task_project, None));
            }
            continue;
        }
        let Some(c) = failed.captures(line) else {
            continue;
        };
        let parts: Vec<&str> = c["chain"].split(" > ").collect();
        let Some((path, _)) = class_path(parts[0]) else {
            continue;
        };
        if !named.contains(&project) {
            named.push(project.clone());
        }
        push(
            &mut out,
            printed(path, project.clone(), gradle_title(&parts)),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_class_name_becomes_its_outermost_classs_path() {
        let at = class_path;
        assert_eq!(
            at("com.acme.FooTest"),
            Some(("com/acme/FooTest.java".into(), None))
        );
        assert_eq!(
            at("com.acme.Outer$Inner.adds"),
            Some(("com/acme/Outer.java".into(), Some("adds".into())))
        );
        assert_eq!(
            at("com.acme.Outer.Inner"),
            Some(("com/acme/Outer.java".into(), Some("Inner".into())))
        );
        assert_eq!(at("CallTest[jvm]"), Some(("CallTest.java".into(), None)));
        assert_eq!(
            at("com.acme.FooTest.sums(int, java.lang.String)[2]"),
            Some(("com/acme/FooTest.java".into(), Some("sums".into())))
        );
        assert_eq!(at("Order tests"), None, "a display name isn't a class");
        assert_eq!(at("com.acme.lowercase"), None);
    }

    #[test]
    fn a_gradle_title_is_the_method_under_nested_classes_and_cases() {
        assert_eq!(gradle_title(&["A", "adds()"]).as_deref(), Some("adds"));
        assert_eq!(
            gradle_title(&["A", "Nested", "adds(int, int)", "[1] 1, 2"]).as_deref(),
            Some("adds")
        );
        assert_eq!(gradle_title(&["A", "adds()[jvm]"]).as_deref(), Some("adds"));
        assert_eq!(
            gradle_title(&["A", "testAdds[0]"]).as_deref(),
            Some("testAdds")
        );
        assert_eq!(project_dir(":core:jvm:test").as_deref(), Some("core/jvm"));
        assert_eq!(project_dir(":test"), None);
    }
}
