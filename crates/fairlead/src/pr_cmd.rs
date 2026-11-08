//! `fairlead pr check`: the `[pr]` rules a change meets before review. In
//! CI it reads the pull request from GitHub's event payload; elsewhere it
//! reads git and, when a pull request exists and `gh` is signed in, its
//! title, body and labels. The pre-push hook and `fairlead done` run it too.

use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use clap::Subcommand;
use fairlead_core::config::{self, LoadOptions, Pr};
use fairlead_guard::fetch;
use fairlead_guard::pr::{self, Facts, Failure, Report};
use serde_json::Value;

/// `gh` asks GitHub over the network; past this the facts it gives are skipped.
const GH_TIMEOUT: Duration = Duration::from_secs(5);
/// A CI runner fetching a missing commit by id.
const CI_FETCH_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Subcommand)]
pub enum PrAction {
    /// Check the change against `[pr]`: from the pull request event in CI,
    /// else from git and, once a pull request exists, from `gh`.
    Check {
        /// Ignore a GitHub event and check what this machine knows.
        #[arg(long)]
        local: bool,
    },
}

pub fn run(action: PrAction, cwd: &Path) -> ExitCode {
    let PrAction::Check { local: force_local } = action;
    let loaded = match config::load(cwd, &LoadOptions::from_process(Vec::new())) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("pr: {e}");
            return ExitCode::from(2);
        }
    };
    if !loaded.problems.is_empty() {
        for p in &loaded.problems {
            eprintln!("{}: {}", p.key, p.message);
        }
        eprintln!("pr: the config has problems; run `fairlead config check`");
        return ExitCode::from(2);
    }
    let root = crate::graph_cmd::repo_root(cwd);
    let Some(rules) = &loaded.config.pr else {
        println!("pr: no [pr] rules configured");
        return ExitCode::SUCCESS;
    };
    let event = (!force_local).then(event_payload).flatten();
    let passed = match event {
        Some(pull) => in_ci(&root, rules, &pull),
        None => checked(rules, local_facts(&root, rules, false, false)),
    };
    if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The pull request in GitHub's event payload, when there is one.
fn event_payload() -> Option<Value> {
    let path = std::env::var_os("GITHUB_EVENT_PATH")?;
    let payload: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    payload
        .get("pull_request")
        .filter(|p| p.is_object())
        .cloned()
}

/// The pre-push hook's half: what this machine knows, the base fetched now.
pub fn local(root: &Path, rules: &Pr) -> bool {
    checked(rules, local_facts(root, rules, true, true))
}

/// Prints the report and says whether every rule that could be checked held.
fn checked(rules: &Pr, facts: Facts) -> bool {
    let report = pr::check(rules, &facts);
    print_report(&report)
}

fn print_report(report: &Report) -> bool {
    if report.failures.is_empty() {
        println!("pr: {} rule(s) passed", report.passed);
        if !report.unchecked.is_empty() {
            print!("{}", report.render());
        }
        return true;
    }
    eprintln!("pr: {} rule(s) failed:", report.failures.len());
    eprint!("{}", report.render());
    false
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether `ancestor` is in `rev`'s history; None when either is missing.
fn contains(root: &Path, ancestor: &str, rev: &str) -> Option<bool> {
    let status = Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, rev])
        .current_dir(root)
        .stderr(Stdio::null())
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// Files added, copied, changed or renamed between `from` and `to`, or the
/// working tree with its untracked files when `to` is None.
fn changed(root: &Path, from: &str, to: Option<&str>) -> Option<Vec<String>> {
    let mut args = vec![
        "diff",
        "--name-only",
        "--diff-filter=ACMR",
        "--no-ext-diff",
        from,
    ];
    args.extend(to);
    let mut files: Vec<String> = git(root, &args)?.lines().map(String::from).collect();
    if to.is_none() {
        let untracked = git(root, &["ls-files", "--others", "--exclude-standard"])?;
        files.extend(untracked.lines().map(String::from));
    }
    Some(files)
}

fn needs_pull_request(rules: &Pr) -> bool {
    let ticket_outside_branch = rules
        .ticket
        .as_ref()
        .is_some_and(|t| t.places.iter().any(|p| *p != config::Place::Branch));
    let labelled = rules.files.iter().any(|f| f.unless_label.is_some());
    rules.title.is_some()
        || ticket_outside_branch
        || !rules.body_headings.is_empty()
        || rules.base.is_some()
        || labelled
        || rules.labels != config::Labels::default()
}

/// What this machine can tell: the branch, the base's history (fetched at a
/// push, else when the last fetch is stale), the changed files, and the pull
/// request's title, body and labels from `gh` when one exists.
pub fn local_facts(root: &Path, rules: &Pr, push: bool, committed_only: bool) -> Facts {
    let mut facts = Facts {
        branch: fairlead_tests::git::branch(root),
        ..Facts::default()
    };
    let remote = fetch::remote_for(root, rules.base.as_deref());
    if rules.base.is_some() && rules.up_to_date {
        if let Some(warning) = fetch::refresh(root, &remote, push) {
            eprintln!("{}", warning.replacen("migrations:", "pr:", 1));
        }
    }
    let base_rev = if git(
        root,
        &["rev-parse", "--verify", "--quiet", &remote.tracking],
    )
    .is_some()
    {
        Some(remote.tracking.clone())
    } else {
        fairlead_tests::git::default_base(root).ok()
    };
    if let Some(base) = &base_rev {
        if rules.base.is_some() {
            facts.contains_base = contains(root, base, "HEAD");
        }
        if let Some(at) = git(root, &["merge-base", base, "HEAD"]) {
            let to = committed_only.then_some("HEAD");
            facts.changed = changed(root, &at, to);
        }
    }
    if needs_pull_request(rules) {
        if let Some(view) = gh_view(root) {
            facts.title = view["title"].as_str().map(String::from);
            facts.body = Some(view["body"].as_str().unwrap_or_default().to_string());
            facts.base = view["baseRefName"].as_str().map(String::from);
            facts.labels = Some(label_names(&view["labels"]));
        }
    }
    facts
}

fn label_names(labels: &Value) -> Vec<String> {
    labels
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["name"].as_str().map(String::from))
        .collect()
}

/// The open pull request for this branch, from `gh`, or None when there's
/// none, `gh` isn't installed or signed in, or it doesn't answer in time.
fn gh_view(root: &Path) -> Option<Value> {
    let mut child = Command::new("gh")
        .args([
            "pr",
            "view",
            "--json",
            "title,body,labels,baseRefName,state",
        ])
        .current_dir(root)
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if start.elapsed() < GH_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let view: Value = serde_json::from_str(&reader.join().ok()?).ok()?;
    (view["state"].as_str() == Some("OPEN")).then_some(view)
}

/// Makes sure a commit is here, fetching it by id when it isn't.
fn have(root: &Path, sha: &str) -> bool {
    let spec = format!("{sha}^{{commit}}");
    if git(root, &["cat-file", "-e", &spec]).is_some() {
        return true;
    }
    let mut fetch = Command::new("git");
    fetch
        .args(["fetch", "--quiet", "--no-tags", "origin", sha])
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0");
    fetch::run_with_timeout(fetch, CI_FETCH_TIMEOUT).is_some()
        && git(root, &["cat-file", "-e", &spec]).is_some()
}

/// CI: the event gives the title, body, labels and both branches; git gives
/// the history between them. Anything left unknown here fails, since CI is
/// the last place to check it.
fn in_ci(root: &Path, rules: &Pr, pull: &Value) -> bool {
    let base_sha = pull["base"]["sha"].as_str().unwrap_or_default();
    let head_sha = pull["head"]["sha"].as_str().unwrap_or_default();
    let mut facts = Facts {
        branch: pull["head"]["ref"].as_str().map(String::from),
        title: pull["title"].as_str().map(String::from),
        body: Some(pull["body"].as_str().unwrap_or_default().to_string()),
        labels: Some(label_names(&pull["labels"])),
        base: pull["base"]["ref"].as_str().map(String::from),
        ..Facts::default()
    };
    let needs_history = rules.up_to_date && rules.base.is_some() || !rules.files.is_empty();
    if needs_history && have(root, base_sha) && have(root, head_sha) {
        facts.contains_base = contains(root, base_sha, head_sha);
        facts.changed = git(root, &["merge-base", base_sha, head_sha])
            .and_then(|at| changed(root, &at, Some(head_sha)));
    }
    let mut report = pr::check(rules, &facts);
    if !report.unchecked.is_empty() {
        report.failures.push(Failure {
            rule: "pr-history",
            what: format!("couldn't check: {}", report.unchecked.join("; ")),
            fix: "check out the pull request with its history (actions/checkout with `fetch-depth: 0`)"
                .into(),
        });
        report.unchecked.clear();
    }
    print_report(&report)
}

/// The `[pr]` rules as lines for `fairlead next`, so they're seen before
/// the pull request is opened.
pub fn summary(rules: &Pr) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(b) = &rules.base {
        out.push(format!(
            "base `{b}`{}",
            if rules.up_to_date {
                ", up to date with it"
            } else {
                ""
            }
        ));
    }
    if let Some(b) = &rules.branch {
        out.push(format!("branch name matches `{b}`"));
    }
    if let Some(t) = &rules.title {
        out.push(format!("title matches `{t}`"));
    }
    if let Some(t) = &rules.ticket {
        let places: Vec<&str> = t.places.iter().map(|p| p.name()).collect();
        let join = if t.any { " or " } else { " and " };
        out.push(format!(
            "ticket key `{}` in the {}",
            t.pattern,
            places.join(join)
        ));
    }
    if !rules.body_headings.is_empty() {
        out.push(format!("body headings: {}", rules.body_headings.join(", ")));
    }
    for f in &rules.files {
        let unless = f
            .unless_label
            .as_deref()
            .map(|l| format!(" (unless labelled `{l}`)"))
            .unwrap_or_default();
        out.push(format!(
            "adds a file matching {}{unless}",
            f.path.join(", ")
        ));
    }
    out
}
