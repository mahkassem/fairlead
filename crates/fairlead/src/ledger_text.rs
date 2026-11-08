//! The ledger in words: the one line that answers "is this red mine?", a
//! line per failure, the default branch's red streaks, and the Markdown
//! `ci report` and the step summary carry. Also the cached ledger the
//! local commands read, only when it's for HEAD and fresh.

use std::path::Path;

use fairlead_replay::ledger::{Failure, Ledger, Verdict};
use fairlead_replay::streaks::Streak;

/// How long a cached ledger speaks for HEAD before it's too old to trust.
pub const FRESH_MINUTES: i64 = 30;

const ORDER: [Verdict; 4] = [
    Verdict::Inherited,
    Verdict::Flaky,
    Verdict::Infra,
    Verdict::Own,
];

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("{n} {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// The evidence most of `of` share, for the summary.
fn common_why(of: &[&Failure]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for f in of {
        match counts.iter_mut().find(|(w, _)| *w == f.why) {
            Some((_, n)) => *n += 1,
            None => counts.push((&f.why, 1)),
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(w, _)| w.to_string())
        .unwrap_or_default()
}

/// One line: what share of the failures each verdict explains, the first
/// with its evidence, such as
/// "2 of 3 failures inherited: red on main since abc1234 (2026-11-07); 1 own".
pub fn summary(l: &Ledger) -> String {
    let total = l.failures.len();
    if total == 0 {
        let running = l.runs.iter().filter(|r| r.conclusion == "in_progress").count();
        return match (l.runs.len(), running) {
            (0, _) => "no CI runs found for this commit".to_string(),
            (_, 0) => format!("no failures in {}", plural(l.runs.len(), "run")),
            (_, n) => format!("no failures so far; {} still running", plural(n, "run")),
        };
    }
    let mut parts = Vec::new();
    for v in ORDER {
        let of: Vec<&Failure> = l.failures.iter().filter(|f| f.verdict == v).collect();
        if of.is_empty() {
            continue;
        }
        if parts.is_empty() {
            let head = if of.len() == total {
                format!("all {} {}", plural(total, "failure"), v.name())
            } else {
                format!("{} of {} {}", of.len(), plural(total, "failure"), v.name())
            };
            let why = match v {
                Verdict::Own => String::new(),
                _ => format!(": {}", common_why(&of)),
            };
            parts.push(format!("{head}{why}"));
        } else {
            parts.push(format!("{} {}", of.len(), v.name()));
        }
    }
    parts.join("; ")
}

fn target(f: &Failure) -> String {
    f.test.clone().unwrap_or_else(|| format!("job {}", f.job))
}

/// A red streak in one line, or none for a workflow that's green.
pub fn streak_line(branch: &str, s: &Streak) -> Option<String> {
    if s.red == 0 {
        return None;
    }
    let mut line = format!(
        "{} ({}) red on {branch} for {}",
        s.workflow,
        s.event,
        plural(s.red, "run")
    );
    if let Some(since) = &s.since {
        line.push_str(&format!(" since {}", since.short()));
    }
    if let Some(h) = s.hours {
        line.push_str(&format!(", {h} h"));
    }
    if s.never_green {
        line.push_str(if s.whole_history {
            "; never green since it was added"
        } else {
            "; no green run among those read"
        });
    }
    for d in &s.drift {
        line.push_str(&format!(
            "; the cause drifted in {}: was \"{}\", now \"{}\"",
            d.job, d.was, d.now
        ));
    }
    Some(line)
}

/// Every red streak, one line each.
pub fn streak_lines(l: &Ledger) -> Vec<String> {
    let branch = l.default_branch.as_deref().unwrap_or("the default branch");
    l.streaks
        .iter()
        .filter_map(|s| streak_line(branch, s))
        .collect()
}

/// What `ci ledger` prints.
pub fn text(l: &Ledger) -> String {
    let mut out = format!(
        "ledger {}  {}\n",
        l.head_sha.chars().take(7).collect::<String>(),
        summary(l)
    );
    for f in &l.failures {
        out.push_str(&format!(
            "  {:<9} {}  {}: {}",
            f.verdict.name(),
            f.job,
            target(f),
            f.why
        ));
        if let Some(r) = &f.rerun {
            out.push_str(&format!(" (rerun: {r})"));
        }
        out.push('\n');
    }
    for line in streak_lines(l) {
        out.push_str(&format!("streak  {line}\n"));
    }
    for p in &l.proposals {
        out.push_str(&format!("propose {}\n", p.entry()));
    }
    out
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// The ledger as a Markdown section, said once for the whole run.
pub fn markdown(l: &Ledger) -> String {
    let mut out = format!("#### Is this red mine?\n\n{}.\n\n", summary(l));
    if !l.failures.is_empty() {
        out.push_str("| Verdict | Job | Failure | Why |\n|---|---|---|---|\n");
        for f in &l.failures {
            let why = match &f.rerun {
                Some(r) => format!("{}; rerun: {r}", f.why),
                None => f.why.clone(),
            };
            out.push_str(&format!(
                "| {} | `{}` | `{}` | {} |\n",
                f.verdict.name(),
                cell(&f.job),
                cell(&target(f)),
                cell(&why)
            ));
        }
        out.push('\n');
    }
    if !l.proposals.is_empty() {
        out.push_str("<details><summary>Quarantine entries the flaky failures back (confirm before adding)</summary>\n\n```toml\n");
        for p in &l.proposals {
            out.push_str(&format!(
                "[[replay.quarantine]]\npath = {:?}\njob = {:?}\nreason = {:?}\nuntil = \"{}\"\n\n",
                p.path, p.job, p.reason, p.until
            ));
        }
        out.push_str("```\n\n</details>\n\n");
    }
    let streaks = streak_lines(l);
    if !streaks.is_empty() {
        out.push_str("Red on the default branch:\n\n");
        for line in streaks {
            out.push_str(&format!("- {line}\n"));
        }
        out.push('\n');
    }
    out
}

/// Minutes since the ledger was made, by the clock now.
pub fn age_minutes(l: &Ledger) -> Option<i64> {
    let now = fairlead_guard::events::timestamp(std::time::SystemTime::now());
    let then = fairlead_replay::window::epoch_seconds(&l.made_at)?;
    Some((fairlead_replay::window::epoch_seconds(&now)? - then).max(0) / 60)
}

/// The cached ledger for HEAD, when one is fresh, with its age in minutes.
pub fn fresh_for_head(root: &Path) -> Option<(Ledger, i64)> {
    let head = crate::ci_cmd::rev_parse(root, "HEAD")?;
    let path = crate::ci_ledger::cache_path(root, &head)?;
    let l = crate::ci_ledger::read(&path).ok()?;
    let age = age_minutes(&l)?;
    (age < FRESH_MINUTES).then_some((l, age))
}

/// The line `next`, `resume` and `brief` print from a fresh cached ledger.
pub fn head_line(root: &Path) -> Option<String> {
    let (l, age) = fresh_for_head(root)?;
    Some(format!(
        "{} (ledger {age} min old; `fairlead ci ledger` refreshes it)",
        summary(&l)
    ))
}

/// The newest cached ledger whatever its commit, for `doctor`'s streaks.
pub fn newest(root: &Path) -> Option<Ledger> {
    let dir = crate::ci_ledger::cache_dir(root)?;
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| crate::ci_ledger::read(&e.path()).ok())
        .max_by(|a, b| a.made_at.cmp(&b.made_at))
}

/// `doctor`'s lines: each red streak the newest ledger recorded, with its age.
pub fn doctor_lines(root: &Path) -> String {
    let Some(l) = newest(root) else {
        return String::new();
    };
    let lines = streak_lines(&l);
    let age = age_minutes(&l).map_or(String::new(), |m| format!(", {m} min old"));
    if lines.is_empty() {
        return if l.default_branch.is_some() {
            format!("streaks: none red on {} (ledger{age})\n", l.default_branch.as_deref().unwrap_or(""))
        } else {
            String::new()
        };
    }
    let mut out = format!("streaks: {} red (ledger{age}; `fairlead ci ledger` refreshes it)\n", lines.len());
    for line in lines {
        out.push_str(&format!("  {line}\n"));
    }
    out
}

#[cfg(test)]
#[path = "ledger_text_tests.rs"]
mod tests;
