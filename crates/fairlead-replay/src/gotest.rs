//! `go test` failures. Each package's output ends with `FAIL\t<import
//! path>`, its failed top-level tests are `--- FAIL: TestX`, and the file is
//! the `x_test.go:12:` a test logged, before its `--- FAIL` under `-v` and
//! after it otherwise, else a `_test.go` frame of a panic, else a compile
//! error in a `_test.go` file. gotestsum's `=== FAIL: <package> <test>` is
//! read too. The file is printed as `/<import path>/<file>`, which
//! attribution matches by its tail like a path on the runner.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::extract::Printed;

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("built-in pattern compiles"))
}

fn top(name: &str) -> String {
    name.split('/').next().unwrap_or(name).to_string()
}

#[derive(Default)]
struct Block {
    failed: Vec<String>,
    logged: HashMap<String, String>,
    frames: Vec<String>,
    compile: Vec<String>,
    current: Option<String>,
}

impl Block {
    fn emit(self, package: &str, out: &mut Vec<Printed>) {
        let under = |file: &str| {
            if file.starts_with('/') || file.contains(':') {
                file.replace('\\', "/")
            } else {
                format!("/{package}/{}", file.rsplit('/').next().unwrap_or(file))
            }
        };
        let fallback = self.frames.first().or(self.compile.first()).cloned();
        let mut push = |printed: Printed| {
            if !out.contains(&printed) {
                out.push(printed);
            }
        };
        if self.failed.is_empty() {
            if let Some(file) = &fallback {
                push(Printed {
                    path: under(file),
                    project: None,
                    title: None,
                });
            }
            return;
        }
        for test in &self.failed {
            let file = self.logged.get(test).or(fallback.as_ref());
            push(Printed {
                path: file.map_or_else(|| format!("/{package}"), |f| under(f)),
                project: None,
                title: Some(test.clone()),
            });
        }
    }
}

pub fn gotest(lines: &[String]) -> Vec<Printed> {
    static STATUS: OnceLock<Regex> = OnceLock::new();
    static LOGGED: OnceLock<Regex> = OnceLock::new();
    static FRAME: OnceLock<Regex> = OnceLock::new();
    static COMPILE: OnceLock<Regex> = OnceLock::new();
    static PACKAGE: OnceLock<Regex> = OnceLock::new();
    static GOTESTSUM: OnceLock<Regex> = OnceLock::new();
    let status = re(
        &STATUS,
        r"^\s*(?:=== (?P<run>RUN|CONT|PAUSE|NAME)|--- (?P<end>FAIL|PASS|SKIP):)\s+(?P<name>\S+)",
    );
    // testify prints `x_test.go:12:` alone, and its message on the lines below.
    let logged = re(&LOGGED, r"^\s+(?P<file>[^\s:]+_test\.go):\d+:(?:\s|$)");
    let frame = re(
        &FRAME,
        r"^\s+(?P<path>(?:[A-Za-z]:)?[/\\]\S+_test\.go):\d+(?:\s+\+0x[0-9a-f]+)?\s*$",
    );
    let compile = re(&COMPILE, r"^(?P<file>\S+_test\.go):\d+:\d+: ");
    let package = re(
        &PACKAGE,
        r"^FAIL\s+(?P<pkg>\S+)(?:\s+\[[\w ]+\]|\s+[\d.]+s)\s*$",
    );
    let gotestsum = re(&GOTESTSUM, r"^=== FAIL: (?P<pkg>\S+) (?P<name>\S+)");
    let mut out = Vec::new();
    let mut block = Block::default();
    // gotestsum's failures, one per package and top-level test, in order.
    let mut summed: Vec<(String, String, Option<String>)> = Vec::new();
    let mut in_summed = false;
    for line in lines {
        if let Some(c) = gotestsum.captures(line) {
            let key = (c["pkg"].to_string(), top(&c["name"]));
            if !summed.iter().any(|(p, n, _)| (p, n) == (&key.0, &key.1)) {
                summed.push((key.0, key.1, None));
            }
            in_summed = true;
            continue;
        }
        if in_summed {
            if line.starts_with("=== ") || line.starts_with("DONE ") {
                in_summed = false;
            } else {
                let found = logged
                    .captures(line)
                    .map(|c| c["file"].to_string())
                    .or_else(|| frame.captures(line).map(|c| c["path"].to_string()));
                if let (Some(found), Some((_, _, file))) = (found, summed.last_mut()) {
                    file.get_or_insert(found);
                }
                continue;
            }
        }
        if let Some(c) = package.captures(line) {
            std::mem::take(&mut block).emit(&c["pkg"], &mut out);
            continue;
        }
        if line.starts_with("ok  ") || line.starts_with("?   ") {
            block = Block::default();
            continue;
        }
        if let Some(c) = status.captures(line) {
            let name = top(&c["name"]);
            if c.name("end").is_some_and(|e| e.as_str() == "FAIL") && !block.failed.contains(&name)
            {
                block.failed.push(name.clone());
            }
            block.current = Some(name);
            continue;
        }
        if let (Some(c), Some(test)) = (logged.captures(line), block.current.clone()) {
            block
                .logged
                .entry(test)
                .or_insert_with(|| c["file"].to_string());
        } else if let Some(c) = frame.captures(line) {
            block.frames.push(c["path"].to_string());
        } else if let Some(c) = compile.captures(line) {
            block.compile.push(c["file"].to_string());
        }
    }
    for (pkg, name, file) in summed {
        let mut b = Block {
            failed: vec![name.clone()],
            ..Block::default()
        };
        if let Some(file) = file {
            b.logged.insert(name, file);
        }
        b.emit(&pkg, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn a_passing_package_logs_nothing_and_a_silent_failure_names_only_its_package() {
        let log = lines(
            "=== RUN   TestOK\n    ok_test.go:5: fine\n--- PASS: TestOK (0.00s)\nok  \texample.com/shop/ok\t0.002s\n--- FAIL: TestQuiet (0.00s)\nFAIL\nFAIL\texample.com/shop/quiet\t0.003s\n",
        );
        assert_eq!(
            gotest(&log),
            [Printed {
                path: "/example.com/shop/quiet".into(),
                project: None,
                title: Some("TestQuiet".into()),
            }]
        );
    }

    #[test]
    fn gotestsum_names_the_package_and_the_test() {
        let log = lines(
            "=== FAIL: sdk/trace TestSpan/child (0.00s)\n    span_test.go:40: wrong parent\n=== FAIL: sdk/trace TestSpan (0.00s)\n\nDONE 12 tests, 2 failures in 1.2s\n",
        );
        assert_eq!(
            gotest(&log),
            [Printed {
                path: "/sdk/trace/span_test.go".into(),
                project: None,
                title: Some("TestSpan".into()),
            }],
            "the subtest's file stands for its parent"
        );
    }

    #[test]
    fn a_testify_failure_names_its_file_on_a_line_of_its_own() {
        let log = lines(
            "--- FAIL: TestParallelCallbacksShutdownStopsWorkers (0.00s)\n    parallel_callbacks_test.go:248:\n        \tError Trace:\t/home/runner/work/otel/otel/sdk/metric/parallel_callbacks_test.go:248\n        \tError:      \tShould be true\nFAIL\nFAIL\tgo.opentelemetry.io/otel/sdk/metric\t0.998s\n",
        );
        assert_eq!(
            gotest(&log),
            [Printed {
                path: "/go.opentelemetry.io/otel/sdk/metric/parallel_callbacks_test.go".into(),
                project: None,
                title: Some("TestParallelCallbacksShutdownStopsWorkers".into()),
            }]
        );
    }
}
