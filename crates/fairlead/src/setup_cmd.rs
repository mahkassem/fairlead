//! `fairlead setup`: applies what is local and reversible (a first config,
//! the agent hooks, the git hook, the agents block, one test file per
//! runner), then prints the checklist of `setup_items` with the one fix for
//! each item still open. `--check` and `--dry-run` change nothing. It never
//! writes a workflow, deletes a file or replaces a config.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::config::{self, Config, ConfigError, LoadOptions, Loaded};
use fairlead_lang::tree::Tree;
use serde_json::json;

use crate::setup_items::{self, Item, State};
use crate::setup_track::{self, Track};
use crate::{agents_cmd, hooks_cmd, init_cmd, init_guard, setup_prove};

#[derive(clap::Args)]
pub struct SetupArgs {
    /// Print the checklist and change nothing.
    #[arg(long, conflicts_with = "dry_run")]
    check: bool,
    /// Print what setup would change, and change nothing.
    #[arg(long)]
    dry_run: bool,
    /// The kind of project this is, over the one detected.
    #[arg(long, value_enum, value_name = "NAME")]
    track: Option<Track>,
    /// Print the checklist as JSON.
    #[arg(long)]
    json: bool,
}

/// The repository as one run reads it.
struct Seen {
    loaded: Result<Loaded, ConfigError>,
    /// Where the config's paths start.
    root: PathBuf,
    /// The repository's top, where the hooks go.
    repo: PathBuf,
    tree: Tree,
}

impl Seen {
    fn read(cwd: &Path) -> Seen {
        let loaded = config::load(cwd, &LoadOptions::from_process(Vec::new()));
        let root = setup_items::root_of(cwd, loaded.as_ref().ok());
        Seen {
            tree: Tree::scan(&root),
            repo: crate::graph_cmd::repo_root(&root),
            root,
            loaded,
        }
    }

    fn loaded(&self) -> Option<&Loaded> {
        self.loaded.as_ref().ok()
    }

    fn config(&self) -> Option<&Config> {
        self.loaded().map(|l| &l.config)
    }

    /// No project file, and none this run could be about to replace.
    fn unwritten(&self) -> bool {
        self.loaded().is_some_and(|l| l.files.is_empty())
            && !self.root.join("fairlead.toml").exists()
    }

    fn items(&self, track: Track) -> Vec<Item> {
        setup_items::check_in(&self.tree, self.loaded(), track)
    }
}

fn is_open(items: &[Item], id: &str) -> bool {
    items.iter().any(|i| i.id == id && i.state == State::Open)
}

pub fn run(args: SetupArgs, cwd: &Path) -> ExitCode {
    let seen = Seen::read(cwd);
    if fairlead_guard::git::git_dir(&seen.repo).is_none() {
        eprintln!("setup: not inside a git repository; run `git init` first");
        return ExitCode::from(2);
    }
    let detected = setup_track::detect(&seen.root, &seen.tree.files, seen.config());
    let (track, why) = match args.track {
        Some(asked) => (
            asked,
            format!(
                "asked for with --track; detected {}: {}",
                detected.track.name(),
                detected.why
            ),
        ),
        None => (detected.track, detected.why),
    };
    let before = seen.items(track);
    let mut would = Vec::new();
    let items = if args.check {
        before
    } else if args.dry_run {
        would = plan(&seen, &before);
        before
    } else {
        apply(cwd, seen, before, track)
    };
    let (open, _) = setup_items::open_of(&items);
    if args.json {
        let mut out = json!({
            "track": { "name": track.name(), "why": why },
            "items": items,
            "open": open,
        });
        if args.dry_run {
            out["would"] = would
                .iter()
                .map(|(id, text)| json!({ "id": id, "text": text }))
                .collect();
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&out).expect("the checklist serializes")
        );
    } else if args.dry_run {
        print!("{}", render_plan(track, &why, &would));
    } else {
        print!("{}", render(track, &why, &items));
    }
    if setup_items::blocked(&items) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn row(word: &str, id: &str, text: &str) -> String {
    format!("  {word:<7}{id:<11}{text}\n")
}

fn header(command: &str, track: Track, why: &str) -> String {
    format!("{command}    track: {} ({why})\n", track.name())
}

/// The checklist: a state, an id and what was found on each line, and the
/// fix under each open item.
fn render(track: Track, why: &str, items: &[Item]) -> String {
    let mut out = header("fairlead setup", track, why);
    for item in items {
        out.push_str(&row(item.state.word(), item.id, &item.text));
        if let (State::Open, Some(fix)) = (item.state, &item.fix) {
            out.push_str(&format!("         fix: {fix}\n"));
        }
    }
    let shows = "`fairlead setup --check` shows this list any time.";
    out.push_str(&match setup_items::open_of(items).0 {
        0 => format!("Nothing open. {shows}\n"),
        1 => format!("1 open. Fix it and run `fairlead setup` again. {shows}\n"),
        n => format!("{n} open. Fix them and run `fairlead setup` again. {shows}\n"),
    });
    out
}

fn render_plan(track: Track, why: &str, would: &[(&'static str, String)]) -> String {
    let mut out = header("fairlead setup --dry-run", track, why);
    for (id, text) in would {
        out.push_str(&row("would", id, text));
    }
    out.push_str(match would.is_empty() {
        true => "Nothing to change. `fairlead setup --check` shows the checklist.\n",
        false => "Nothing was changed. `fairlead setup` applies these.\n",
    });
    out
}

/// What `apply` would do, an item each, in the order it would.
fn plan(seen: &Seen, items: &[Item]) -> Vec<(&'static str, String)> {
    let mut would = Vec::new();
    // The config init would write isn't loaded here, so what follows it
    // is planned from the defaults.
    let mut full = seen.config().is_some_and(|c| c.profile.is_full());
    let writes = is_open(items, "config") && seen.unwritten();
    if writes {
        let (_, guard) = init_cmd::plain(&seen.root, &seen.tree.files);
        full = !guard;
        let runners = init_cmd::runners_in(&seen.root, &seen.tree.files);
        let text = match (guard, runners.as_slice()) {
            (true, _) => "the guard only: init finds no test runner here".to_string(),
            (false, []) => "the checks init finds, and no test runner".to_string(),
            (false, ids) => format!("the runners init finds: {}", ids.join(", ")),
        };
        would.push(("config", format!("write fairlead.toml with {text}")));
    }
    let Some(config) = seen.config() else {
        return would;
    };
    if is_open(items, "hooks") {
        let names: Vec<&str> = hooks_cmd::agents(&seen.repo)
            .iter()
            .filter(|a| a.stages.is_empty())
            .map(|a| a.name)
            .collect();
        would.push((
            "hooks",
            format!("install the hooks for {}", names.join(", ")),
        ));
    }
    if is_open(items, "git-hook") {
        let (_, written, _) = setup_items::commit_stage(&seen.repo);
        let text = match (written, init_guard::on_path("lefthook")) {
            (false, true) => "add the commit stage through lefthook, then run `lefthook install`",
            (false, false) => "add the commit stage through lefthook; it stays inactive until lefthook is installed",
            (true, true) => "run `lefthook install`",
            (true, false) => "nothing: lefthook isn't on the PATH, so the commit stage stays inactive",
        };
        would.push(("git-hook", text.to_string()));
    }
    if is_open(items, "agents") {
        let files: Vec<&str> = config
            .agents
            .files
            .items()
            .iter()
            .map(String::as_str)
            .collect();
        would.push((
            "agents",
            format!("write the fairlead block in {}", files.join(", ")),
        ));
    }
    if writes && full {
        let text = "run the smallest test file of each runner the new config names";
        would.push(("proved", text.to_string()));
    } else if full {
        let chosen = setup_prove::plan(&seen.repo, &seen.tree, config).unwrap_or_default();
        for c in chosen {
            let text = format!("run {} on {}: {}", c.runner, c.file, c.inv.argv.join(" "));
            would.push(("proved", text));
        }
    }
    would
}

/// `lefthook install`, so the commit stage runs.
fn activate(repo: &Path) -> Result<(), String> {
    let out = fairlead_core::process::command("lefthook", repo)
        .arg("install")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("`lefthook install` didn't start: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    let last = said.lines().rev().find(|l| !l.trim().is_empty());
    Err(format!(
        "`lefthook install` failed: {}",
        last.unwrap_or("no output").trim()
    ))
}

/// Applies each step whose item is open, in order, then checks again: an
/// item this run closed reads `fixed`, and one it couldn't says why.
fn apply(cwd: &Path, mut seen: Seen, mut items: Vec<Item>, track: Track) -> Vec<Item> {
    let mut tried: Vec<&'static str> = Vec::new();
    let mut failed: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut attempt = |id: &'static str, result: Result<(), String>| {
        tried.push(id);
        if let Err(e) = result {
            failed.insert(id, e);
        }
    };
    if is_open(&items, "config") && seen.unwritten() {
        let (text, _) = init_cmd::plain(&seen.root, &seen.tree.files);
        attempt(
            "config",
            init_guard::land(&seen.root, &text).map_err(|lines| lines.join("; ")),
        );
        seen = Seen::read(cwd);
        items = seen.items(track);
    }
    // A config that doesn't load is fixed by hand before anything is built on it.
    if let Some(config) = seen.config() {
        if is_open(&items, "hooks") {
            attempt(
                "hooks",
                hooks_cmd::install_agents(&seen.repo, config).map(drop),
            );
        }
        if is_open(&items, "git-hook") {
            let written = hooks_cmd::install_git_quietly(&seen.repo, config).map(drop);
            let active = written.and_then(|()| match init_guard::on_path("lefthook") {
                true => activate(&seen.repo),
                false => Ok(()),
            });
            attempt("git-hook", active);
        }
        if is_open(&items, "agents") {
            attempt("agents", agents_cmd::sync(&seen.root, config).map(drop));
        }
        if config.profile.is_full() && !seen.unwritten() {
            match setup_prove::prove(&seen.repo, &seen.tree, config, setup_prove::LIMIT) {
                Ok(0) => {}
                Ok(_) => attempt("proved", Ok(())),
                Err(e) => attempt("proved", Err(e)),
            }
        }
    }
    // Read again from disk, so the list is what this run left behind.
    let mut after = setup_items::check(&seen.root, seen.loaded(), track);
    for item in &mut after {
        match (item.state, failed.get(item.id)) {
            (State::Ok, _) if tried.contains(&item.id) => item.state = State::Fixed,
            (State::Open, Some(why)) => {
                item.text = format!("{}; setup couldn't fix it: {why}", item.text)
            }
            _ => {}
        }
    }
    after
}
