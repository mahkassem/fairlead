//! The catalogue of setup items: one list that `fairlead setup`, `setup
//! --check`, `doctor` and the session-start line read. Each item says what
//! was found and, when it is open, the one command or edit that closes it.
//! Checking reads files and the event log; it never runs a test or a need.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fairlead_core::config::{AgentsWrite, Config, Loaded, Profile};
use fairlead_core::pattern::Pattern;
use fairlead_guard::lefthook;
use fairlead_lang::tree::Tree;
use fairlead_tests::modules::Modules;
use serde::Serialize;
use serde_json::Value;

use crate::knowledge::lesson::{days, today};
use crate::setup_track::Track;
use crate::{agents_cmd, doctor_version, done_record, hooks_cmd, score_cmd, setup_hints};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Ok,
    /// Open before this run of `setup`, which closed it.
    Fixed,
    Open,
    /// Not due yet, or not possible on this machine.
    Later,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Ok => "ok",
            State::Fixed => "fixed",
            State::Open => "open",
            State::Later => "later",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    /// Stable: binary, config, runners, hooks, git-hook, agents, proved, ci,
    /// smoke, gate, measure, and tests in the guard profile.
    pub id: &'static str,
    pub state: State,
    /// What was found, numbers first.
    pub text: String,
    /// Exactly one command or one config edit, for an open item.
    pub fix: Option<String>,
    /// An open required item makes the exit code 1.
    pub required: bool,
}

/// A runner with more test files than this names `smoke`.
const SMOKE_OVER: usize = 50;
const FRESH_DAYS: i64 = 30;
/// The gate runs carrying `reach` that the scorecard needs.
const MEASURE_RUNS: usize = 10;
const WAITS: &str = "waits for a config that loads";
const INSTALLER: &str = if cfg!(windows) {
    "powershell -c \"irm https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.ps1 | iex\""
} else {
    "curl -fsSL https://github.com/mahkassem/fairlead/releases/latest/download/fairlead-installer.sh | sh"
};

fn ok(id: &'static str, text: impl Into<String>) -> Item {
    Item {
        id,
        state: State::Ok,
        text: text.into(),
        fix: None,
        required: false,
    }
}

fn later(id: &'static str, text: impl Into<String>) -> Item {
    Item {
        state: State::Later,
        ..ok(id, text)
    }
}

fn open(id: &'static str, text: impl Into<String>, fix: impl Into<String>) -> Item {
    Item {
        state: State::Open,
        fix: Some(fix.into()),
        ..ok(id, text)
    }
}

fn plural(n: usize, one: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    }
}

/// Whether an open `id` fails `setup` on this track.
fn required(id: &str, track: Track) -> bool {
    match id {
        "binary" | "config" | "runners" | "hooks" | "git-hook" | "agents" => true,
        "ci" => track == Track::Ongoing,
        _ => false,
    }
}

/// Where the config's paths start: the folder of the project file, or the
/// repository's top where there is none.
pub fn root_of(dir: &Path, loaded: Option<&Loaded>) -> PathBuf {
    match loaded {
        Some(l) if !l.files.is_empty() => l.root.clone(),
        _ => crate::graph_cmd::repo_root(dir),
    }
}

/// Every item for the repository at `root`, in the order they print.
/// `loaded` is none when the config doesn't load.
pub fn check(root: &Path, loaded: Option<&Loaded>, track: Track) -> Vec<Item> {
    check_in(&Tree::scan(root), loaded, track)
}

/// `check`, for a caller that has scanned the tree already.
pub fn check_in(tree: &Tree, loaded: Option<&Loaded>, track: Track) -> Vec<Item> {
    let root = tree.root.as_path();
    let repo = crate::graph_cmd::repo_root(root);
    let config = loaded.map(|l| &l.config);
    // Without a project file the profile isn't chosen yet, so only the
    // items both profiles share are listed.
    let written = config.filter(|_| loaded.is_some_and(|l| !l.files.is_empty()));
    let planned = written.filter(|c| c.profile.is_full());
    let suites = planned.map(|c| Suites::read(tree, c));
    let events = events(&repo);
    let mut items = vec![binary(&repo, config), config_item(loaded)];
    if let (Some(c), Some(s)) = (planned, &suites) {
        items.push(runners(c, s));
    }
    items.push(hooks(&repo));
    items.push(git_hook(&repo));
    items.push(config.map_or_else(|| later("agents", WAITS), |c| agents(root, c)));
    if let (Some(c), Some(s)) = (planned, &suites) {
        items.push(proved(c, s, &crate::setup_prove::read(&repo)));
    }
    items.push(config.map_or_else(|| later("ci", WAITS), |c| ci(&repo, c, track)));
    if let (Some(c), Some(s)) = (planned, &suites) {
        let per_file = crate::done_budget::per_file_of(&events);
        items.push(smoke(c, s, &per_file));
    }
    items.push(gate(&events, days(&today()).unwrap_or(0)));
    match (planned, written) {
        (Some(_), _) => items.push(measure(&events)),
        (None, Some(c)) => items.push(tests(root, c)),
        (None, None) => {}
    }
    // Without lefthook the commit stage can't be made active here, and the
    // agent hooks already guard each edit, so its absence doesn't fail setup.
    let no_lefthook = !crate::init_guard::on_path("lefthook");
    for item in &mut items {
        item.required = required(item.id, track) && !(item.id == "git-hook" && no_lefthook);
    }
    in_order(items, track)
}

/// What a track puts off is printed last.
fn in_order(mut items: Vec<Item>, track: Track) -> Vec<Item> {
    if matches!(track, Track::New | Track::Legacy) {
        items.sort_by_key(|i| i.id == "ci");
    }
    items
}

/// How many items are open, and the first one's id.
pub fn open_of(items: &[Item]) -> (usize, Option<&'static str>) {
    let mut open = items.iter().filter(|i| i.state == State::Open);
    let first = open.next().map(|i| i.id);
    (open.count() + usize::from(first.is_some()), first)
}

/// Whether an open item is one this track requires.
pub fn blocked(items: &[Item]) -> bool {
    items.iter().any(|i| i.required && i.state == State::Open)
}

fn reload(dir: &Path) -> Option<Loaded> {
    use fairlead_core::config::{load, LoadOptions};
    load(dir, &LoadOptions::from_process(Vec::new())).ok()
}

/// The track and each item's state by id: all a report says of the setup.
pub fn states(dir: &Path) -> (&'static str, Vec<(&'static str, &'static str)>) {
    let loaded = reload(dir);
    let root = root_of(dir, loaded.as_ref());
    let tree = Tree::scan(&root);
    let config = loaded.as_ref().map(|l| &l.config);
    let track = crate::setup_track::detect(&root, &tree.files, config).track;
    let items = check_in(&tree, loaded.as_ref(), track);
    let states = items.iter().map(|i| (i.id, i.state.word())).collect();
    (track.name(), states)
}

/// What a new session is told when a required item is open: one line, and
/// one look, a day for a repository. Nothing is proved or changed.
pub fn session_line(dir: &Path) -> Option<String> {
    let loaded = reload(dir);
    let root = root_of(dir, loaded.as_ref());
    let stamp = fairlead_guard::git::common_dir(&root)?
        .join("fairlead")
        .join("setup-told");
    let day = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400)
        .to_string();
    if std::fs::read_to_string(&stamp).is_ok_and(|told| told.trim() == day) {
        return None;
    }
    let tree = Tree::scan(&root);
    let config = loaded.as_ref().map(|l| &l.config);
    let track = crate::setup_track::detect(&root, &tree.files, config).track;
    let items = check_in(&tree, loaded.as_ref(), track);
    // Stamped either way, so a large tree is read once a day at most.
    if let Some(parent) = stamp.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&stamp, &day);
    if !blocked(&items) {
        return None;
    }
    let open: Vec<&str> = (items.iter())
        .filter(|i| i.required && i.state == State::Open)
        .map(|i| i.id)
        .collect();
    Some(format!(
        "fairlead: setup has {} open ({}); run `fairlead setup`, which fixes what it can and names one fix for each of the rest",
        open.len(),
        open.join(", ")
    ))
}

/// What CI says about the setup a checkout shows: the config, the runners,
/// the agents block, and the agent hooks where the repository commits its
/// agent settings. What only a machine has is left out.
pub fn ci_line(dir: &Path) -> Option<String> {
    let loaded = reload(dir);
    let root = root_of(dir, loaded.as_ref());
    let tree = Tree::scan(&root);
    let config = loaded.as_ref().map(|l| &l.config);
    let track = crate::setup_track::detect(&root, &tree.files, config).track;
    let repo = crate::graph_cmd::repo_root(&root);
    let commits_hooks = repo.join(".claude").join("settings.json").exists();
    let seen = |id: &str| match id {
        "config" | "runners" | "agents" => true,
        "hooks" => commits_hooks,
        _ => false,
    };
    let open: Vec<&str> = check_in(&tree, loaded.as_ref(), track)
        .iter()
        .filter(|i| i.state == State::Open && seen(i.id))
        .map(|i| i.id)
        .collect();
    if open.is_empty() {
        return None;
    }
    Some(format!(
        "setup has {} open in the repository ({}); run `fairlead setup` and commit what it writes",
        open.len(),
        open.join(", ")
    ))
}

/// The last line of `fairlead doctor`; none outside a git repository,
/// where there is nothing to set up and no bound on the tree to read.
pub fn doctor_line(dir: &Path, loaded: Option<&Loaded>) -> String {
    let root = root_of(dir, loaded);
    if fairlead_guard::git::git_dir(&root).is_none() {
        return String::new();
    }
    let tree = Tree::scan(&root);
    let track = crate::setup_track::detect(&root, &tree.files, loaded.map(|l| &l.config)).track;
    match open_of(&check_in(&tree, loaded, track)) {
        (n, Some(first)) => format!(
            "setup: {n} open (first: {first}); `fairlead setup --check` lists each with its fix\n"
        ),
        _ => "setup: nothing open\n".to_string(),
    }
}

fn events(repo: &Path) -> Vec<Value> {
    fairlead_guard::events::EventLog::open(repo)
        .map(|log| log.read())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn install_command(runner: Option<&str>) -> &'static str {
    match runner {
        Some("bun x") => "bun install",
        Some("pnpm exec") => "pnpm install",
        Some("yarn") => "yarn install",
        Some(_) => "npm install",
        None => INSTALLER,
    }
}

fn binary(repo: &Path, config: Option<&Config>) -> Item {
    let this = env!("CARGO_PKG_VERSION");
    let floor = config.and_then(|c| c.fairlead.clone());
    let seen = doctor_version::read(repo, floor.as_deref());
    let runner = hooks_cmd::package_runner(repo);
    let mut wrong: Vec<String> = doctor_version::differs(this, &seen)
        .into_iter()
        .map(|(what, _)| what)
        .collect();
    if doctor_version::below_floor(this, &seen) {
        wrong.push(format!(
            "the config needs {} or later and this binary is {this}",
            floor.as_deref().unwrap_or_default()
        ));
    }
    if let Some(first) = wrong.into_iter().next() {
        return open("binary", first, install_command(runner));
    }
    if let Some(spec) = doctor_version::older_pin(this, &seen) {
        return open(
            "binary",
            format!("package.json pins {spec} and this binary is {this}"),
            "fairlead migrate --write",
        );
    }
    if runner.is_some() && seen.installed.is_none() {
        return open(
            "binary",
            format!("package.json lists fairlead and node_modules has no copy of it; this binary is {this}"),
            install_command(runner),
        );
    }
    let whence = match (runner, crate::on_path()) {
        (Some(runner), _) => format!(", the project's own copy through `{runner}`"),
        (None, Some(_)) => " on PATH".to_string(),
        (None, None) => ", not on PATH, where the hooks look for it".to_string(),
    };
    let pinned = match (&seen.spec, doctor_version::pinned_here(this, &seen)) {
        (Some(_), true) => ", matches package.json".to_string(),
        (Some(spec), false) => format!(", within package.json's `{spec}`"),
        (None, _) => String::new(),
    };
    ok("binary", format!("{this}{whence}{pinned}"))
}

fn config_item(loaded: Option<&Loaded>) -> Item {
    let Some(l) = loaded else {
        return open("config", "the config doesn't load", "fairlead config check");
    };
    let Some(file) = l.files.first() else {
        return open(
            "config",
            "no fairlead.toml; defaults apply",
            "fairlead init",
        );
    };
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match l.problems.first() {
        Some(p) => open(
            "config",
            format!(
                "{} in {name}, first: {}: {}",
                plural(l.problems.len(), "problem"),
                p.key,
                p.message
            ),
            "fairlead config check",
        ),
        None if l.config.profile.is_full() => ok("config", format!("{name}, no problems")),
        None => ok("config", format!("{name}, no problems, guard profile")),
    }
}

/// Each runner's test files, and the ones without exactly one runner.
struct Suites {
    /// By the runner's place in the config.
    files: Vec<Vec<String>>,
    tests: usize,
    unmatched: Vec<String>,
    ambiguous: Vec<(String, Vec<String>)>,
    error: Option<String>,
}

impl Suites {
    fn read(tree: &Tree, config: &Config) -> Suites {
        let mut files = vec![Vec::new(); config.tests.runners.items().len()];
        let found = fairlead_tests::testfiles::discover(tree, config, &Modules::default());
        let (found, error) = match found {
            Ok(found) => (found, None),
            Err(e) => (Default::default(), Some(e)),
        };
        for test in &found.tests {
            if let Some(mine) = test.runner.and_then(|i| files.get_mut(i)) {
                mine.push(test.path.clone());
            }
        }
        Suites {
            files,
            tests: found.tests.len(),
            unmatched: found.unmatched,
            ambiguous: found.ambiguous,
            error,
        }
    }
}

fn runners(config: &Config, suites: &Suites) -> Item {
    let count = config.tests.runners.items().len();
    if let Some(e) = &suites.error {
        return open("runners", e.clone(), "fairlead config check");
    }
    if count == 0 {
        return match suites.tests {
            0 => ok("runners", "no test file matches `tests.match` yet"),
            n => open(
                "runners",
                format!("{} and no [[tests.runners]]", plural(n, "test file")),
                "add the runner `fairlead init --dry-run` prints to fairlead.toml",
            ),
        };
    }
    if let Some(path) = suites.unmatched.first() {
        return open(
            "runners",
            format!(
                "{} with no runner, first {path}",
                plural(suites.unmatched.len(), "test file")
            ),
            format!("add a pattern for `{path}` to one runner's `match` in fairlead.toml"),
        );
    }
    if let Some((path, ids)) = suites.ambiguous.first() {
        return open(
            "runners",
            format!(
                "{} with more than one runner, first {path} ({})",
                plural(suites.ambiguous.len(), "test file"),
                ids.join(", ")
            ),
            format!("leave `{path}` to one runner with `exclude` on the other in fairlead.toml"),
        );
    }
    if let Some(warning) = setup_hints::runner_warnings(config).first() {
        // The warning ends with the edit that fixes it.
        let (what, fix) = warning
            .rsplit_once("; ")
            .unwrap_or((warning, "name `{files}` in the runner's command"));
        return open("runners", what, format!("{fix} in fairlead.toml"));
    }
    ok(
        "runners",
        format!(
            "{}, {}, each with exactly one runner",
            plural(count, "runner"),
            plural(suites.tests, "test file")
        ),
    )
}

fn proved(config: &Config, suites: &Suites, record: &crate::setup_prove::Record) -> Item {
    let ids: Vec<&str> = config
        .tests
        .runners
        .items()
        .iter()
        .zip(&suites.files)
        .filter(|(_, files)| !files.is_empty())
        .map(|(r, _)| r.id.as_str())
        .collect();
    if ids.is_empty() {
        return later("proved", "no runner has a test file to run yet");
    }
    let of = |id: &str| record.proved.iter().find(|p| p.runner == id);
    let why =
        |p: &crate::setup_prove::Proof| p.why.clone().unwrap_or_else(|| "no reason kept".into());
    let known: Vec<_> = ids.iter().filter_map(|id| of(id)).collect();
    if let Some(p) = known.iter().find(|p| p.passed == Some(false)) {
        let text = format!("{} failed on {}: {}", p.runner, p.file, why(p));
        let fix = match why(p).starts_with("could not start") {
            true => "install what the runner needs (the project's dependencies, or the service it talks to), then `fairlead setup`",
            false => "fix the failing test or the runner's command, then `fairlead setup`",
        };
        return open("proved", text, fix);
    }
    let unproved: Vec<&str> = ids.iter().copied().filter(|id| of(id).is_none()).collect();
    if !unproved.is_empty() {
        let text = format!(
            "{} of {} {} run a test file here: {}",
            unproved.len(),
            plural(ids.len(), "runner"),
            match unproved.len() {
                1 => "hasn't",
                _ => "haven't",
            },
            unproved.join(", ")
        );
        return open(
            "proved",
            text,
            "fairlead setup (without `--check`, which only lists)",
        );
    }
    if let Some(p) = known.iter().find(|p| p.passed.is_none()) {
        return later("proved", format!("{} can't run here: {}", p.runner, why(p)));
    }
    let each: Vec<String> = known
        .iter()
        .map(|p| match p.seconds {
            s if s < 0.05 => format!("{} under 0.1 s", p.runner),
            s => format!("{} {s:.1} s", p.runner),
        })
        .collect();
    ok(
        "proved",
        format!(
            "{} ran one test file: {}",
            plural(ids.len(), "runner"),
            each.join(", ")
        ),
    )
}

fn hooks(repo: &Path) -> Item {
    let agents = hooks_cmd::agents(repo);
    let without: Vec<&hooks_cmd::AgentHooks> =
        agents.iter().filter(|a| a.stages.is_empty()).collect();
    if let Some(first) = without.first() {
        let names: Vec<&str> = without.iter().map(|a| a.name).collect();
        return open(
            "hooks",
            format!("no hooks for {}", names.join(", ")),
            first.install.clone(),
        );
    }
    let each: Vec<String> = agents
        .iter()
        .map(|a| format!("{}: {}", a.name, a.stages.join(", ")))
        .collect();
    ok("hooks", each.join("; "))
}

/// The lefthook config's name, whether it holds the commit stage, and
/// whether lefthook is in the git hooks to run it.
pub fn commit_stage(repo: &Path) -> (String, bool, bool) {
    let file = hooks_cmd::lefthook_file(repo);
    let shown = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("lefthook.yml")
        .to_string();
    let written = std::fs::read_to_string(&file).is_ok_and(|text| lefthook::present(&text));
    let active = fairlead_guard::git::git_dir(repo).is_some_and(|d| hooks_cmd::lefthook_active(&d));
    (shown, written, active)
}

fn git_hook(repo: &Path) -> Item {
    match commit_stage(repo) {
        (shown, true, true) => ok(
            "git-hook",
            format!("the commit stage is in {shown}, and lefthook runs it"),
        ),
        (shown, true, false) => {
            let (missing, fix) = match crate::init_guard::on_path("lefthook") {
                true => ("", "lefthook install"),
                false => (
                    ", and lefthook isn't on the PATH",
                    "install lefthook (it is what runs the commit stage), then `lefthook install`",
                ),
            };
            open(
                "git-hook",
                format!("written but not active: {shown} has the commit stage, .git/hooks doesn't run lefthook{missing}"),
                fix,
            )
        }
        (shown, false, _) => open(
            "git-hook",
            format!("no commit stage in {shown}"),
            "fairlead hooks install --git",
        ),
    }
}

fn agents(root: &Path, config: &Config) -> Item {
    if config.agents.write == AgentsWrite::Never {
        return ok(
            "agents",
            "`agents.write = \"never\"`: a person keeps the block",
        );
    }
    let (files, wrong) = agents_cmd::status(root, config);
    match wrong.as_slice() {
        [] if files.is_empty() => ok("agents", "`agents.files` is empty"),
        [] => ok(
            "agents",
            format!("the block is current in {}", files.join(", ")),
        ),
        [one] => open("agents", one.clone(), "fairlead agents sync"),
        [first, ..] => open(
            "agents",
            format!(
                "{} without a current block, first: {first}",
                plural(wrong.len(), "file")
            ),
            "fairlead agents sync",
        ),
    }
}

fn ci(repo: &Path, config: &Config, track: Track) -> Item {
    let profile = config.profile;
    // The key says CI collects what the gate owes; with no job doing it, nothing does.
    if config.ci.settle && profile == Profile::Full && !score_cmd::ci_settles(repo) {
        return open(
            "ci",
            format!(
                "`ci.settle` is on, and no workflow runs `fairlead ci settle`, so what the gate owes is never collected (for a workflow of your own: {})",
                crate::ci_settle_cmd::PAGE
            ),
            "fairlead ci workflow --write",
        );
    }
    let (what, fix) = match profile {
        Profile::Full => ("`fairlead ci`", "fairlead ci workflow --write"),
        Profile::Guard => (
            "`fairlead guard check`",
            "add a step that runs `fairlead guard check` to a workflow",
        ),
    };
    if score_cmd::ci_runs(repo, profile) {
        let settles = match config.ci.settle {
            true => ", and `fairlead ci settle` collects what the gate owes",
            false => "",
        };
        return ok("ci", format!("a workflow runs {what}{settles}"));
    }
    match track {
        Track::New | Track::Legacy => later(
            "ci",
            format!(
                "no workflow runs {what}; the {} track leaves it for later",
                track.name()
            ),
        ),
        _ => open("ci", format!("no workflow runs {what}"), fix),
    }
}

fn smoke(config: &Config, suites: &Suites, per_file: &BTreeMap<String, f64>) -> Item {
    let budget = f64::from(config.done.budget_seconds);
    // A suite the gate has timed and that fits the budget runs whole.
    let fits = |id: &str, files: usize| {
        let whole = per_file.get(id).map(|t| t * files as f64);
        whole.is_some_and(|w| budget > 0.0 && w <= budget)
    };
    let needs: Vec<(&str, Vec<Pattern>)> = config
        .needs
        .items()
        .iter()
        .map(|n| {
            let globs = n.tests.iter().filter_map(|g| Pattern::new(g).ok());
            (n.id.as_str(), globs.collect())
        })
        .collect();
    let behind = |files: &[String]| {
        needs
            .iter()
            .find(|(_, globs)| files.iter().any(|f| globs.iter().any(|p| p.is_match(f))))
            .map(|(id, _)| *id)
    };
    // Each runner that wants `smoke`, with why.
    let wanting: Vec<(&fairlead_core::config::Runner, String)> = config
        .tests
        .runners
        .items()
        .iter()
        .zip(&suites.files)
        .filter_map(|(runner, files)| {
            if files.len() > SMOKE_OVER && !fits(&runner.id, files.len()) {
                return Some((runner, plural(files.len(), "test file")));
            }
            behind(files).map(|need| (runner, format!("need \"{need}\" covers it")))
        })
        .collect();
    if let Some((runner, why)) = wanting.iter().find(|(r, _)| r.smoke.is_empty()) {
        return open(
            "smoke",
            format!("{}: {why}, and it names no `smoke`", runner.id),
            format!(
                "name 1 to 3 test files under `smoke` on runner `{}` in fairlead.toml",
                runner.id
            ),
        );
    }
    // A `smoke` entry that names no file of its runner stands for nothing.
    let runners = config.tests.runners.items().iter().zip(&suites.files);
    for (runner, files) in runners {
        let stale = runner
            .smoke
            .iter()
            .find(|glob| Pattern::new(glob).is_ok_and(|p| !files.iter().any(|f| p.is_match(f))));
        if let Some(glob) = stale {
            return open(
                "smoke",
                format!(
                    "{}: its `smoke` entry `{glob}` names no test file it matches",
                    runner.id
                ),
                format!(
                    "name test files that exist under `smoke` on runner `{}` in fairlead.toml",
                    runner.id
                ),
            );
        }
    }
    let ids: Vec<&str> = wanting.iter().map(|(r, _)| r.id.as_str()).collect();
    match ids.len() {
        0 => ok(
            "smoke",
            format!(
                "no runner has a need, or over {SMOKE_OVER} test files that outlast the budget"
            ),
        ),
        _ => ok("smoke", format!("`smoke` is named on {}", ids.join(", "))),
    }
}

fn gate(events: &[Value], today: i64) -> Item {
    let last = events
        .iter()
        .filter(|e| e["stage"] == "done")
        .filter(|e| done_record::state(Some(e)) == done_record::State::Passed)
        .filter_map(|e| days(e["at"].as_str()?.get(..10)?).map(|at| today - at))
        .min();
    match last {
        Some(d) if d <= 0 => ok("gate", "`fairlead done` last passed today"),
        Some(d) if d <= FRESH_DAYS => ok(
            "gate",
            format!(
                "`fairlead done` last passed {} ago",
                plural(d as usize, "day")
            ),
        ),
        Some(d) => open(
            "gate",
            format!("`fairlead done` last passed {d} days ago"),
            "fairlead done",
        ),
        None => open(
            "gate",
            "no passing `fairlead done` on record",
            "fairlead done",
        ),
    }
}

fn measure(events: &[Value]) -> Item {
    let runs = events
        .iter()
        .filter(|e| e["stage"] == "done" && e["reach"].is_object())
        .count();
    match MEASURE_RUNS.saturating_sub(runs) {
        0 => ok(
            "measure",
            format!("{runs} gate runs carry `reach`, enough for the scorecard"),
        ),
        more => later(
            "measure",
            format!(
                "the scorecard needs {more} more {}",
                match more {
                    1 => "gate run",
                    _ => "gate runs",
                }
            ),
        ),
    }
}

fn tests(root: &Path, config: &Config) -> Item {
    match setup_hints::appeared(root, config) {
        Some((what, fix)) => open("tests", what, fix),
        None => ok("tests", "no tests a runner here can run yet"),
    }
}

#[cfg(test)]
#[path = "setup_items_tests.rs"]
mod tests;
