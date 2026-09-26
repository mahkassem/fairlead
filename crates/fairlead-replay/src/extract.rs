//! Failing test files named in a CI log. Each runner prints them its own
//! way: Vitest as `FAIL  [project]  path > suite > test`, Jest as
//! `FAIL path (1.2 s)` with the test's title on a following `●` line, and
//! either may sit behind a workspace runner's `<package> <script>: ` prefix.
//! Bun names the file once, on a `path:` header it prints again whenever
//! parallel output switches files, and each failure as `(fail) suite > test`
//! under it; its closing summary repeats the failures with no header, so
//! nothing is read again until the next header. Lines inside an assertion
//! diff (`+`/`-`) are another run's output, so they're skipped.

use std::sync::OnceLock;

use regex::Regex;

/// A failing test file as the log printed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printed {
    pub path: String,
    /// The runner's project, or the workspace runner's package prefix.
    pub project: Option<String>,
    /// The last segment of the failing test's name, when printed.
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Extractor {
    Vitest,
    Jest,
    Bun,
    /// A pattern with a named `file` group, and optionally `project` and `title`.
    Regex(Regex),
}

impl Extractor {
    pub fn named(name: &str, pattern: Option<&str>) -> Result<Extractor, String> {
        match (name, pattern) {
            ("vitest", _) => Ok(Extractor::Vitest),
            ("jest", _) => Ok(Extractor::Jest),
            ("bun", _) => Ok(Extractor::Bun),
            ("regex", Some(p)) => {
                let re = Regex::new(p).map_err(|e| format!("replay.failures pattern: {e}"))?;
                if re.capture_names().flatten().all(|n| n != "file") {
                    return Err("replay.failures pattern needs a named `file` group".into());
                }
                Ok(Extractor::Regex(re))
            }
            ("regex", None) => Err("replay.failures: extractor \"regex\" needs a pattern".into()),
            (other, _) => Err(format!(
                "unknown extractor `{other}`; use vitest, jest, bun or regex"
            )),
        }
    }
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("built-in pattern compiles"))
}

const TEST_EXT: &str = r"\.(?:[cm]?[jt]sx?)";

pub(crate) fn clean(line: &str) -> String {
    static STAMP: OnceLock<Regex> = OnceLock::new();
    static ANSI: OnceLock<Regex> = OnceLock::new();
    let line = re(&ANSI, r"\x1b\[[0-9;]*[A-Za-z]").replace_all(line, "");
    re(&STAMP, r"^\d{4}-\d\d-\d\dT[\d:.]+Z ")
        .replace(&line, "")
        .into_owned()
}

/// `<package> <script>: rest`, as `pnpm -r` and similar runners print it.
fn split_prefix(line: &str) -> (Option<String>, &str) {
    static PREFIX: OnceLock<Regex> = OnceLock::new();
    let prefix = re(&PREFIX, r"^(\S+) [\w:.-]+: (.*)$");
    match prefix.captures(line) {
        Some(c) if !line.trim_start().starts_with("FAIL") => {
            let package = c
                .get(1)
                .map(|m| m.as_str().to_string())
                .filter(|p| p != ".");
            (package, c.get(2).map_or(line, |m| m.as_str()))
        }
        _ => (None, line),
    }
}

fn last_segment(title: &str, separator: &str) -> Option<String> {
    let segment = title.rsplit(separator).next()?.trim();
    (!segment.is_empty()).then(|| segment.to_string())
}

pub fn extract(extractor: &Extractor, log: &str) -> Vec<Printed> {
    let lines: Vec<String> = log.lines().map(clean).collect();
    if let Extractor::Bun = extractor {
        return bun(&lines);
    }
    let mut out: Vec<Printed> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let (package, line) = split_prefix(raw);
        if line.trim_start().starts_with(['+', '-']) {
            continue;
        }
        let found = match extractor {
            Extractor::Vitest => vitest(line),
            Extractor::Jest => jest(line).map(|mut p| {
                p.title = jest_title(&lines[i + 1..]);
                p
            }),
            Extractor::Regex(re) => custom(re, line),
            Extractor::Bun => unreachable!("handled above"),
        };
        if let Some(mut printed) = found {
            printed.project = printed.project.or(package);
            if !out.contains(&printed) {
                out.push(printed);
            }
        }
    }
    out
}

fn vitest(line: &str) -> Option<Printed> {
    static FAIL: OnceLock<Regex> = OnceLock::new();
    let pattern = format!(
        r"^\s*FAIL\s+(?:\|(?P<pipe>[^|]+)\|\s+|(?P<bare>\S+)\s{{2,}})?(?P<path>[^\s>\[]+{TEST_EXT})(?::\d+)?(?:\s+>\s+(?P<title>.*?))?(?:\s+\[.*\])?\s*$"
    );
    let c = re(&FAIL, &pattern).captures(line)?;
    Some(Printed {
        path: c["path"].to_string(),
        project: c
            .name("pipe")
            .or(c.name("bare"))
            .map(|m| m.as_str().trim().to_string()),
        title: c
            .name("title")
            .and_then(|t| last_segment(t.as_str(), " > ")),
    })
}

fn jest(line: &str) -> Option<Printed> {
    static FAIL: OnceLock<Regex> = OnceLock::new();
    let pattern = format!(
        r"^\s*FAIL\s+(?:(?P<project>\S+)\s{{2,}})?(?P<path>\S+{TEST_EXT})(?:\s+\([\d.]+ m?s\))?\s*$"
    );
    let c = re(&FAIL, &pattern).captures(line)?;
    Some(Printed {
        path: c["path"].to_string(),
        project: c.name("project").map(|m| m.as_str().to_string()),
        title: None,
    })
}

/// The first `● suite › test` after a Jest FAIL line, before the next file.
fn jest_title(after: &[String]) -> Option<String> {
    for raw in after.iter().take(40) {
        let (_, line) = split_prefix(raw);
        let line = line.trim_start();
        if line.starts_with("FAIL") || line.starts_with("PASS") {
            return None;
        }
        if let Some(title) = line.strip_prefix('●') {
            return last_segment(title, " › ");
        }
    }
    None
}

/// A test file's header, as bun prints it or as GitHub renders its group.
pub fn bun_header(line: &str) -> Option<&str> {
    static HEADER: OnceLock<Regex> = OnceLock::new();
    let pattern = format!(r"^(?:##\[group\]|::group::)?(?P<path>[^\s(][^:]*?{TEST_EXT}):\s*$");
    re(&HEADER, &pattern)
        .captures(line)
        .and_then(|c| c.name("path"))
        .map(|m| m.as_str())
}

fn bun(lines: &[String]) -> Vec<Printed> {
    static FAIL: OnceLock<Regex> = OnceLock::new();
    static SUMMARY: OnceLock<Regex> = OnceLock::new();
    let fail = re(
        &FAIL,
        r"^\s*\(fail\)\s+(?P<title>.*?)(?:\s+\[[\d.]+m?s\])?\s*$",
    );
    let summary = re(&SUMMARY, r"^\s*\d+ tests? failed:\s*$");
    let mut file: Option<&str> = None;
    let mut out: Vec<Printed> = Vec::new();
    for line in lines {
        // A job can run bun more than once; the next run's headers start over.
        if summary.is_match(line) {
            file = None;
            continue;
        }
        if let Some(path) = bun_header(line) {
            file = Some(path);
            continue;
        }
        let (Some(path), Some(c)) = (file, fail.captures(line)) else {
            continue;
        };
        let title = last_segment(&c["title"], " > ").filter(|t| t != "(unnamed)");
        let printed = Printed {
            path: path.to_string(),
            project: None,
            title,
        };
        if !out.contains(&printed) {
            out.push(printed);
        }
    }
    out
}

fn custom(re: &Regex, line: &str) -> Option<Printed> {
    let c = re.captures(line)?;
    Some(Printed {
        path: c.name("file")?.as_str().to_string(),
        project: c.name("project").map(|m| m.as_str().to_string()),
        title: c.name("title").map(|m| m.as_str().to_string()),
    })
}
