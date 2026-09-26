//! `fairlead guard compare` and `fairlead guard bench`: how an adopter checks
//! the guard against the linter it replaces, and how fast the write hook is
//! on the edits their history actually made.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::OnceLock;
use std::time::Instant;

use fairlead_core::config::{self, LoadOptions};
use fairlead_guard::Guard;
use regex::Regex;
use serde_json::{json, Value};

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("guard: {message}");
    ExitCode::FAILURE
}

type Key = (String, u32, String);

fn line_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^(?:\./)?(?P<file>[^\s:][^:]*):(?P<line>[0-9]+)(?::[0-9]+)?:? +(?P<rule>[A-Za-z0-9_./@-]+?):?(?:\s|$)")
            .expect("valid")
    })
}

/// `file:line rule`, with an optional column and anything after the rule.
fn parse(output: &str, maps: &BTreeMap<String, String>) -> Vec<Key> {
    output
        .lines()
        .filter_map(|l| line_pattern().captures(l.trim_end()))
        .filter_map(|c| {
            let rule = &c["rule"];
            let rule = maps.get(rule).map_or(rule, String::as_str);
            Some((
                c["file"].replace('\\', "/"),
                c["line"].parse().ok()?,
                rule.to_string(),
            ))
        })
        .collect()
}

fn tally(keys: impl IntoIterator<Item = Key>) -> BTreeMap<Key, usize> {
    let mut counts = BTreeMap::new();
    for key in keys {
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

/// What `a` has more of than `b`, one entry per extra finding.
fn extra(a: &BTreeMap<Key, usize>, b: &BTreeMap<Key, usize>) -> Vec<Key> {
    a.iter()
        .flat_map(|(k, &n)| {
            std::iter::repeat_n(k.clone(), n.saturating_sub(b.get(k).copied().unwrap_or(0)))
        })
        .collect()
}

pub fn compare(
    root: &Path,
    guard: &Guard,
    file: &str,
    rules: &[String],
    maps: &[String],
) -> ExitCode {
    let mut map = BTreeMap::new();
    for m in maps {
        let Some((theirs, ours)) = m.split_once('=') else {
            return fail(format!("--map takes THEIRS=OURS, got `{m}`"));
        };
        map.insert(theirs.trim().to_string(), ours.trim().to_string());
    }
    let mut text = String::new();
    let read = if file == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(file).map(|t| text = t)
    };
    if let Err(e) = read {
        return fail(format!("{file}: {e}"));
    }
    let keep = |k: &Key| rules.is_empty() || rules.contains(&k.2);
    let theirs: Vec<Key> = parse(&text, &map).into_iter().filter(keep).collect();
    let ours: Vec<Key> = match crate::guard_cmd::tree_findings(root, guard, None) {
        Ok(found) => found
            .into_iter()
            .map(|f| (f.file, f.line, f.rule.to_string()))
            .filter(keep)
            .collect(),
        Err(e) => return fail(e),
    };
    let (t, o) = (tally(theirs.clone()), tally(ours.clone()));
    let (only_theirs, only_ours) = (extra(&t, &o), extra(&o, &t));
    println!(
        "guard compare: {} finding(s) from the other linter, {} from Fairlead",
        theirs.len(),
        ours.len()
    );
    if only_theirs.is_empty() && only_ours.is_empty() {
        println!("guard compare: the same, finding for finding");
        return ExitCode::SUCCESS;
    }
    for (label, keys) in [
        ("only the other linter", &only_theirs),
        ("only Fairlead", &only_ours),
    ] {
        if keys.is_empty() {
            continue;
        }
        println!("{} {label}:", keys.len());
        for (file, line, rule) in keys.iter().take(20) {
            println!("  {file}:{line} {rule}");
        }
        if keys.len() > 20 {
            println!("  and {} more", keys.len() - 20);
        }
    }
    ExitCode::FAILURE
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Nearest rank, so the value is one that was measured.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// A worktree removed when the bench ends, however it ends.
struct Worktree {
    repo: PathBuf,
    path: PathBuf,
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let path = self.path.to_string_lossy().into_owned();
        let _ = git(&self.repo, &["worktree", "remove", "--force", &path]);
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn run(cwd: &Path, since: &str, limit: usize, p95_under: Option<f64>) -> ExitCode {
    let loaded = match config::load(cwd, &LoadOptions::from_process(Vec::new())) {
        Ok(loaded) => loaded,
        Err(e) => return fail(e),
    };
    if !loaded.problems.is_empty() {
        return fail("the config has problems; run `fairlead config check`");
    }
    let repo = crate::graph_cmd::repo_root(cwd);
    let root = if loaded.files.is_empty() {
        repo.clone()
    } else {
        loaded.root.clone()
    };
    let guard = match Guard::new(&loaded.config.guard, &root) {
        Ok(guard) if !guard.is_empty() => guard,
        Ok(_) => return fail("no rules configured, so there's nothing to bench"),
        Err(e) => return fail(e),
    };
    let Some(git_dir) = fairlead_guard::git::git_dir(&repo) else {
        return fail("not inside a git repository");
    };
    let commits = match git(
        &repo,
        &[
            "rev-list",
            "--reverse",
            "--first-parent",
            &format!("{since}..HEAD"),
        ],
    ) {
        Ok(out) => out.lines().map(String::from).collect::<Vec<_>>(),
        Err(e) => return fail(e),
    };
    let path = git_dir
        .join("fairlead")
        .join(format!("bench-{}", std::process::id()));
    let path_arg = path.to_string_lossy().into_owned();
    if let Err(e) = git(
        &repo,
        &["worktree", "add", "--quiet", "--detach", &path_arg, "HEAD"],
    ) {
        return fail(e);
    }
    let tree = Worktree {
        repo: repo.clone(),
        path,
    };
    let project = tree
        .path
        .join(root.strip_prefix(&repo).unwrap_or(Path::new("")));
    // Today's config, whatever the config was at each commit.
    let configs: Vec<(PathBuf, Vec<u8>)> = loaded
        .files
        .iter()
        .filter_map(|f| Some((project.join(f.file_name()?), std::fs::read(f).ok()?)))
        .collect();
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return fail(e),
    };
    let mut times = Vec::new();
    let mut decisions: BTreeMap<&str, usize> = BTreeMap::new();
    let mut used = 0;
    for commit in &commits {
        if times.len() >= limit {
            break;
        }
        let parent = format!("{commit}^");
        if let Err(e) = git(
            &tree.path,
            &["checkout", "--quiet", "--force", "--detach", &parent],
        ) {
            return fail(e);
        }
        for (file, bytes) in &configs {
            if let Err(e) = std::fs::write(file, bytes) {
                return fail(format!("{}: {e}", file.display()));
            }
        }
        let prefix = root
            .strip_prefix(&repo)
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .replace('\\', "/");
        let changed = match git(
            &tree.path,
            &["diff", "--name-only", "--diff-filter=AM", &parent, commit],
        ) {
            Ok(out) => out,
            Err(e) => return fail(e),
        };
        let mut any = false;
        for rel in changed.lines() {
            if times.len() >= limit {
                break;
            }
            let in_project = if prefix.is_empty() {
                Some(rel)
            } else {
                rel.strip_prefix(&format!("{prefix}/"))
            };
            let Some(project_rel) = in_project.filter(|p| guard.reads(p)) else {
                continue;
            };
            let Ok(text) = git(&tree.path, &["show", &format!("{commit}:{rel}")]) else {
                continue;
            };
            let call = json!({
                "cwd": project.to_string_lossy(),
                "hook_event_name": "PreToolUse",
                "tool_name": "Write",
                "tool_input": { "file_path": project.join(project_rel).to_string_lossy(), "content": text },
            });
            let start = Instant::now();
            let answer = match hook(&exe, &project, &call) {
                Ok(answer) => answer,
                Err(e) => return fail(e),
            };
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            any = true;
            let decision = match answer.as_ref().map(|a| &a["hookSpecificOutput"]) {
                Some(out) if out.get("permissionDecision").is_some() => "deny",
                Some(_) => "warn",
                None => "allow",
            };
            *decisions.entry(decision).or_insert(0) += 1;
        }
        used += usize::from(any);
    }
    if times.is_empty() {
        println!("guard bench: no commit since {since} changed a file the guard reads");
        return ExitCode::SUCCESS;
    }
    times.sort_by(f64::total_cmp);
    let (p50, p95, max) = (
        percentile(&times, 50.0),
        percentile(&times, 95.0),
        times[times.len() - 1],
    );
    let counts: Vec<String> = decisions.iter().map(|(d, n)| format!("{n} {d}")).collect();
    println!(
        "guard bench: {} edit(s) from {used} commit(s) through the write hook",
        times.len()
    );
    println!(
        "  wall time, process start included: p50 {p50:.1} ms, p95 {p95:.1} ms, max {max:.1} ms"
    );
    println!("  decisions: {}", counts.join(", "));
    match p95_under {
        Some(budget) if p95 > budget => fail(format!("p95 {p95:.1} ms is over {budget} ms")),
        _ => ExitCode::SUCCESS,
    }
}

/// One call through the hook binary, as Claude Code makes it.
fn hook(exe: &Path, dir: &Path, call: &Value) -> Result<Option<Value>, String> {
    let mut child = Command::new(exe)
        .args(["guard", "hook"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(call.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(serde_json::from_str(text.trim()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_linters_lines_are_read_as_file_line_rule() {
        let maps: BTreeMap<String, String> =
            [("max-lines".to_string(), "file-length".to_string())].into();
        let keys = parse(
            "src/a.ts:3 history: comment carries a date\n./src/b.ts:10:4: max-lines too long\nsrc\\c.ts:1 density\nsummary: 3\n",
            &maps,
        );
        assert_eq!(
            keys,
            [
                ("src/a.ts".into(), 3, "history".into()),
                ("src/b.ts".into(), 10, "file-length".into()),
                ("src/c.ts".into(), 1, "density".into()),
            ]
        );
    }

    #[test]
    fn extra_counts_each_finding_one_side_has_more_of() {
        let key = |l| ("a".to_string(), l, "r".to_string());
        let a = tally([key(1), key(1), key(2)]);
        let b = tally([key(1), key(3)]);
        assert_eq!(extra(&a, &b), [key(1), key(2)]);
        assert_eq!(extra(&b, &a), [key(3)]);
    }
}
