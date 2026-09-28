//! Graph providers: the built-in JavaScript and TypeScript scanner, and
//! external commands that print a graph for the files they claim, for any
//! language or build tool the scanner doesn't read.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use fairlead_core::config::GraphProvider;
use fairlead_core::pattern::Pattern;
use fairlead_core::provider::{ProviderOutput, VERSION};

use crate::graph::{EdgeKind, Graph};

/// The built-in scanner's id, in reports.
pub const BUILTIN: &str = "typescript";

/// What one provider contributed to a graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: String,
    /// Files it claimed.
    pub files: usize,
    pub edges: usize,
    /// Edges it printed that name a file outside the tree or outside its claim.
    pub ignored: usize,
    /// Why it failed, when it did; its files then count as uncertain.
    pub failed: Option<String>,
}

/// A file two providers' patterns both matched, and which one won.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub file: String,
    pub chosen: String,
    pub other: String,
}

/// How specific a glob is: the length of its literal part before the first
/// wildcard, so `src/go/**` beats `**/*.go`.
fn specificity(glob: &str) -> usize {
    glob.find(['*', '?', '[', '{']).unwrap_or(glob.len())
}

/// Which external provider claims each file, by the most specific matching
/// pattern, and every file two providers both matched.
pub fn claims(
    files: &[String],
    providers: &[GraphProvider],
) -> Result<(HashMap<String, usize>, Vec<Conflict>), String> {
    let compiled: Vec<Vec<(Pattern, usize)>> = providers
        .iter()
        .map(|p| {
            p.files
                .iter()
                .map(|g| Ok((Pattern::new(g)?, specificity(g))))
                .collect::<Result<Vec<_>, String>>()
        })
        .collect::<Result<_, _>>()?;
    let mut owner = HashMap::new();
    let mut conflicts = Vec::new();
    for file in files {
        let matched: Vec<(usize, usize)> = compiled
            .iter()
            .enumerate()
            .filter_map(|(i, pats)| {
                pats.iter()
                    .filter(|(p, _)| p.is_match(file))
                    .map(|(_, s)| *s)
                    .max()
                    .map(|s| (i, s))
            })
            .collect();
        // Most specific first; the earlier provider wins a tie.
        let Some(&(best, _)) = matched
            .iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        else {
            continue;
        };
        for &(other, _) in matched.iter().filter(|(i, _)| *i != best) {
            conflicts.push(Conflict {
                file: file.clone(),
                chosen: providers[best].id.clone(),
                other: providers[other].id.clone(),
            });
        }
        owner.insert(file.clone(), best);
    }
    Ok((owner, conflicts))
}

/// Runs one external provider on the files it claims and adds its edges.
pub fn run(root: &Path, provider: &GraphProvider, claimed: &[String], graph: &mut Graph) -> Report {
    let mut report = Report {
        id: provider.id.clone(),
        files: claimed.len(),
        edges: 0,
        ignored: 0,
        failed: None,
    };
    let output = match output(root, provider, claimed) {
        Ok(o) => o,
        Err(e) => {
            report.failed = Some(e);
            return report;
        }
    };
    let own: std::collections::HashSet<&str> = claimed.iter().map(String::as_str).collect();
    for edge in output.edges {
        match (graph.id(&edge.from), graph.id(&edge.to)) {
            (Some(from), Some(to)) if own.contains(edge.from.as_str()) => {
                graph.add_edge(from, to, EdgeKind::Provider);
                report.edges += 1;
            }
            _ => report.ignored += 1,
        }
    }
    report
}

fn output(
    root: &Path,
    provider: &GraphProvider,
    claimed: &[String],
) -> Result<ProviderOutput, String> {
    let (program, args) = provider.command.split_first().ok_or("no command")?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start {program}: {e}"))?;
    let list = claimed.join("\n") + "\n";
    let mut stdin = child.stdin.take().expect("stdin is piped");
    // Written on a thread, so a provider that prints before reading can't deadlock.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(list.as_bytes());
    });
    let out = wait(
        child,
        std::time::Duration::from_secs(provider.timeout_seconds),
    )
    .map_err(|e| format!("{program} {e}"))?;
    let _ = writer.join();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().next().unwrap_or("").trim();
        return Err(format!("{program} exited with {}: {first}", out.status));
    }
    let parsed: ProviderOutput = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("its output isn't a provider graph: {e}"))?;
    if parsed.version != VERSION {
        return Err(format!(
            "it printed version {}; this fairlead reads version {VERSION}",
            parsed.version
        ));
    }
    Ok(parsed)
}

/// The child's output, or an error once it has run for `limit`: a hung
/// provider must fail the graph, not hang the plan.
fn wait(
    mut child: std::process::Child,
    limit: std::time::Duration,
) -> Result<std::process::Output, String> {
    let read = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    };
    let stdout = read(Box::new(child.stdout.take().expect("stdout is piped")));
    let stderr = read(Box::new(child.stderr.take().expect("stderr is piped")));
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("ran past its {} s timeout", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(e) => return Err(format!("couldn't be waited for: {e}")),
        }
    };
    Ok(std::process::Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, files: &[&str]) -> GraphProvider {
        GraphProvider {
            id: id.into(),
            command: vec!["true".into()],
            files: files.iter().map(|s| s.to_string()).collect(),
            timeout_seconds: 120,
        }
    }

    #[test]
    fn the_most_specific_pattern_claims_a_file_and_the_conflict_is_kept() {
        let files = vec!["src/go/a.go".to_string(), "lib/b.go".into(), "c.ts".into()];
        let ps = [
            provider("go", &["**/*.go"]),
            provider("vendored", &["src/go/**"]),
        ];
        let (owner, conflicts) = claims(&files, &ps).unwrap();
        assert_eq!(owner["src/go/a.go"], 1);
        assert_eq!(owner["lib/b.go"], 0);
        assert!(!owner.contains_key("c.ts"));
        assert_eq!(
            conflicts,
            [Conflict {
                file: "src/go/a.go".into(),
                chosen: "vendored".into(),
                other: "go".into()
            }]
        );
    }
}
