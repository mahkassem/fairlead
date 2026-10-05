//! `fairlead hooks install | status | uninstall`: the Claude Code hook in
//! `.claude/settings.json` (shared) or `.claude/settings.local.json`, or the
//! same hooks for Codex in `.codex/hooks.json`. Install
//! keeps a manifest in the git directory with the file's original bytes and
//! the bytes it wrote, so uninstall can put the file back exactly.

use std::path::Path;
use std::process::ExitCode;

use clap::Subcommand;
use fairlead_core::config::{self, HooksTarget, LoadOptions};
use fairlead_guard::lefthook;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Runs the write stage, and does nothing where `fairlead` isn't installed,
/// so a teammate without it can still work.
const COMMAND: &str = "command -v fairlead >/dev/null 2>&1 && fairlead guard hook || true";
/// Where the npm package unpacks the binary; calling it skips the package
/// runner's own start-up on every edit.
const NPM_BINARY: &str = "node_modules/fairlead/node_modules/.bin_real/fairlead";
/// How an installed entry is recognised, whatever else is in the command.
const MARK: &str = "fairlead guard hook";
const STOP_MARK: &str = "fairlead guard stop";
const NUDGE_MARK: &str = "fairlead guard nudge";
/// Seconds; the Stop hook hashes the working tree, which a large one makes slower.
const STOP_TIMEOUT: u64 = 30;
const EDIT_TOOLS: &str = "Edit|Write|MultiEdit";
/// Codex names its one editing tool; it also lets `Edit|Write` select it.
const CODEX_EDIT_TOOLS: &str = "apply_patch|Edit|Write";
/// Where each agent's hook starts from: Claude Code says, and Codex starts a
/// hook in the session's directory, which can be below the root.
const CLAUDE_DIR: &str = "${CLAUDE_PROJECT_DIR:-.}";
const CODEX_DIR: &str = "$(git rev-parse --show-toplevel 2>/dev/null || pwd)";
/// Seconds; the hook keeps to its own much shorter budget.
const TIMEOUT: u64 = 10;

#[derive(Subcommand)]
pub enum HooksAction {
    /// Add the Claude Code hooks (the guard before an edit, the brief nudge
    /// after one, the Stop hook) and the git hook when lefthook is set up;
    /// `--codex` adds the same hooks for Codex.
    Install {
        #[command(flatten)]
        target: Target,
    },
    /// Say where the hooks are and whether uninstall can restore each file exactly.
    Status {
        #[command(flatten)]
        target: Target,
    },
    /// Remove the hooks, restoring each file byte for byte when nobody changed it since.
    Uninstall {
        #[command(flatten)]
        target: Target,
    },
}

#[derive(clap::Args)]
pub struct Target {
    /// Only the Claude Code hooks.
    #[arg(long, conflicts_with = "git")]
    claude: bool,
    /// The same hooks for Codex, in `.codex/hooks.json`, instead of Claude Code's.
    #[arg(long, conflicts_with_all = ["claude", "git", "local", "shared"])]
    codex: bool,
    /// Only the git pre-commit hook, through lefthook; install makes a
    /// `lefthook.yml` when there's none.
    #[arg(long)]
    git: bool,
    /// `.claude/settings.local.json`, over `hooks.claude`.
    #[arg(long, conflicts_with = "shared")]
    local: bool,
    /// `.claude/settings.json`, over `hooks.claude`.
    #[arg(long)]
    shared: bool,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    /// The file's bytes before install, or none when it didn't exist.
    original: Option<String>,
    /// The bytes install wrote.
    written: String,
    /// Whether install made the `.claude` directory.
    made_dir: bool,
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("hooks: {message}");
    ExitCode::FAILURE
}

pub fn run(action: HooksAction, cwd: &Path) -> ExitCode {
    let loaded = match config::load(cwd, &LoadOptions::from_process(Vec::new())) {
        Ok(loaded) => loaded,
        Err(e) => return fail(e),
    };
    let root = crate::graph_cmd::repo_root(cwd);
    let Some(git_dir) = fairlead_guard::git::git_dir(&root) else {
        return fail("not inside a git repository");
    };
    let (HooksAction::Install { target }
    | HooksAction::Status { target }
    | HooksAction::Uninstall { target }) = &action;
    let kind = if target.local {
        HooksTarget::Local
    } else if target.shared {
        HooksTarget::Shared
    } else {
        loaded.config.hooks.claude
    };
    let name = match kind {
        HooksTarget::Shared => "settings.json",
        HooksTarget::Local => "settings.local.json",
    };
    let (file, manifest_name) = if target.codex {
        (
            root.join(".codex").join("hooks.json"),
            "codex-hooks.json".to_string(),
        )
    } else {
        (root.join(".claude").join(name), format!("claude-{name}"))
    };
    let manifest = git_dir.join("fairlead").join("backups").join(manifest_name);
    let bash = !loaded.config.guard.commands.items().is_empty();
    let stop = loaded.config.done.on_stop != fairlead_core::config::OnStop::Off;
    let nudge = loaded.config.brief.nudge;
    let backups = git_dir.join("fairlead").join("backups");
    let lefthook = lefthook_file(&root);
    let lefthook_manifest = backups.join(format!(
        "lefthook-{}",
        lefthook
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("lefthook.yml")
    ));
    let claude = !target.git;
    // Without `--git`, install only touches a lefthook config that's already there.
    let git = target.git
        || (!target.claude
            && !target.codex
            && (lefthook.exists() || !matches!(action, HooksAction::Install { .. })));
    let agent = if target.codex {
        Agent::Codex
    } else {
        Agent::Claude
    };
    let runner = package_runner(&root);
    let mut result = Ok(());
    if claude {
        result = match action {
            HooksAction::Install { .. } => install(
                &file,
                &manifest,
                Stages { bash, stop, nudge },
                runner,
                agent,
            ),
            HooksAction::Status { .. } => status(&file, &manifest),
            HooksAction::Uninstall { .. } => uninstall(&file, &manifest),
        };
    }
    if git && result.is_ok() {
        result = match action {
            HooksAction::Install { .. } => {
                git_install(&lefthook, &lefthook_manifest, &git_dir, runner)
            }
            HooksAction::Status { .. } => git_status(&lefthook, &lefthook_manifest, &git_dir),
            HooksAction::Uninstall { .. } => git_uninstall(&lefthook, &lefthook_manifest),
        };
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

/// What `doctor` says about both hooks, one line each.
pub fn describe(root: &Path, config: &fairlead_core::config::Config) -> Vec<String> {
    let name = match config.hooks.claude {
        HooksTarget::Shared => "settings.json",
        HooksTarget::Local => "settings.local.json",
    };
    let file = root.join(".claude").join(name);
    let claude = match read(&file).ok().flatten().map(|t| parse(&file, &t)) {
        Some(Ok(settings)) if !installed(&settings).is_empty() => {
            format!(
                "claude hook: installed in .claude/{name} for {}",
                installed(&settings).join(", ")
            )
        }
        Some(Err(e)) => format!("claude hook: can't read it: {e}"),
        _ => format!(
            "claude hook: not installed in .claude/{name}; `fairlead hooks install` adds it"
        ),
    };
    let lefthook = lefthook_file(root);
    let shown = lefthook
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("lefthook.yml");
    let git = match read(&lefthook).ok().flatten() {
        Some(text) if lefthook::present(&text) => {
            let active = fairlead_guard::git::git_dir(root).is_some_and(|d| lefthook_active(&d));
            if active {
                format!("git hook: installed in {shown}, and lefthook runs it")
            } else {
                format!("git hook: installed in {shown}, but lefthook isn't in .git/hooks; run `lefthook install`")
            }
        }
        _ => "git hook: not installed; `fairlead hooks install --git` adds it through lefthook"
            .to_string(),
    };
    let mut lines = vec![claude, git];
    // Only shown where Codex hooks are, so a Claude Code project isn't told about Codex.
    let codex = root.join(".codex").join("hooks.json");
    if let Some(Ok(settings)) = read(&codex).ok().flatten().map(|t| parse(&codex, &t)) {
        if !installed(&settings).is_empty() {
            lines.push(format!(
                "codex hook: installed in .codex/hooks.json for {}",
                installed(&settings).join(", ")
            ));
        }
    }
    lines
}

/// How a repository that lists Fairlead as a package dependency runs its
/// own copy, told by its lockfile: that copy isn't on the PATH.
pub fn package_runner(root: &Path) -> Option<&'static str> {
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    let manifest: Value = serde_json::from_str(&text).ok()?;
    let listed = ["dependencies", "devDependencies", "optionalDependencies"]
        .iter()
        .any(|k| manifest.get(k).and_then(|d| d.get("fairlead")).is_some());
    if !listed {
        return None;
    }
    let has = |name: &str| root.join(name).is_file();
    Some(if has("bun.lock") || has("bun.lockb") {
        "bun x"
    } else if has("pnpm-lock.yaml") {
        "pnpm exec"
    } else if has("yarn.lock") {
        "yarn"
    } else {
        "npx --no-install"
    })
}

/// The write stage's command: the project's own copy where it has one, else
/// `fairlead` on the PATH. Either way it does nothing where neither runs.
/// The Stop and PostToolUse hooks' command, for `fairlead guard <stage>`.
/// Unlike the write hook's it keeps the exit code and output: the Stop
/// hook's exit 2 and the PostToolUse note are how they reach the agent.
fn stage_command(stage: &str, runner: Option<&str>, dir: &str) -> String {
    match runner {
        None => {
            format!("command -v fairlead >/dev/null 2>&1 || exit 0; exec fairlead guard {stage}")
        }
        Some(runner) => format!(
            "cd \"{dir}\" 2>/dev/null || exit 0; \
             if [ -x {NPM_BINARY} ]; then exec {NPM_BINARY} guard {stage}; fi; \
             exec {runner} fairlead guard {stage}"
        ),
    }
}

fn claude_command(runner: Option<&str>, dir: &str) -> String {
    match runner {
        None => COMMAND.to_string(),
        Some(runner) => format!(
            "cd \"{dir}\" 2>/dev/null || exit 0; \
             if [ -x {NPM_BINARY} ]; then {NPM_BINARY} guard hook; \
             else {runner} fairlead guard hook 2>/dev/null; fi; exit 0"
        ),
    }
}

/// The lefthook config lefthook would read, or where a new one goes.
pub fn lefthook_file(root: &Path) -> std::path::PathBuf {
    lefthook::FILES
        .iter()
        .map(|n| root.join(n))
        .find(|p| p.is_file())
        .unwrap_or_else(|| root.join(lefthook::FILES[0]))
}

/// Whether lefthook has put its runner in the git hooks, so the entry runs.
pub fn lefthook_active(git_dir: &Path) -> bool {
    std::fs::read_to_string(git_dir.join("hooks").join("pre-commit"))
        .is_ok_and(|t| t.contains("lefthook"))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn keep_manifest(manifest: &Path, record: &Manifest) -> Result<(), String> {
    // The first manifest is the one that knows the file before Fairlead.
    if manifest.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(manifest.parent().expect("a manifest has a directory"))
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    write(
        manifest,
        &serde_json::to_string_pretty(record).expect("manifest serializes"),
    )
}

fn git_install(
    file: &Path,
    manifest: &Path,
    git_dir: &Path,
    runner: Option<&str>,
) -> Result<(), String> {
    let original = read(file)?;
    let run = match runner {
        Some(runner) => format!("{runner} {}", lefthook::RUN),
        None => lefthook::RUN.to_string(),
    };
    let written = match lefthook::insert(original.as_deref().unwrap_or(""), &run) {
        lefthook::Insert::Already => {
            println!("hooks: already in {}", file.display());
            return Ok(());
        }
        lefthook::Insert::Refused(why) => return Err(format!("{}: {why}", file.display())),
        lefthook::Insert::Text(text) => text,
    };
    keep_manifest(
        manifest,
        &Manifest {
            original,
            written: written.clone(),
            made_dir: false,
        },
    )?;
    write(file, &written)?;
    println!("hooks: added the commit stage to {}", file.display());
    if !lefthook_active(git_dir) {
        println!("hooks: lefthook isn't in .git/hooks yet; run `lefthook install` so it runs");
    }
    Ok(())
}

fn git_status(file: &Path, manifest: &Path, git_dir: &Path) -> Result<(), String> {
    let Some(text) = read(file)? else {
        println!("hooks: no lefthook config, so no commit stage");
        return Ok(());
    };
    if !lefthook::present(&text) {
        println!("hooks: the commit stage isn't in {}", file.display());
        return Ok(());
    }
    let exact = read_manifest(manifest)?.is_some_and(|m| m.written == text);
    let restore = if exact {
        "byte for byte"
    } else {
        "by removing only its lines"
    };
    println!(
        "hooks: the commit stage is in {}; uninstall restores it {restore}",
        file.display()
    );
    if !lefthook_active(git_dir) {
        println!("hooks: lefthook isn't in .git/hooks, so it won't run; run `lefthook install`");
    }
    Ok(())
}

fn git_uninstall(file: &Path, manifest_path: &Path) -> Result<(), String> {
    let current = read(file)?;
    let manifest = read_manifest(manifest_path)?;
    // Removed only once the file is dealt with, so a failed write keeps the original.
    let done = || {
        let _ = std::fs::remove_file(manifest_path);
    };
    if let (Some(text), Some(m)) = (&current, &manifest) {
        if *text == m.written {
            match &m.original {
                Some(original) => write(file, original)?,
                None => {
                    std::fs::remove_file(file).map_err(|e| format!("{}: {e}", file.display()))?
                }
            }
            done();
            println!(
                "hooks: removed the commit stage; {} is back as it was, byte for byte",
                file.display()
            );
            return Ok(());
        }
    }
    let Some(text) = current else {
        done();
        println!("hooks: no lefthook config, so no commit stage");
        return Ok(());
    };
    match lefthook::remove(&text) {
        Some(left) => {
            write(file, &left)?;
            println!(
                "hooks: removed the commit stage's lines from {}",
                file.display()
            );
        }
        None => println!("hooks: the commit stage isn't in {}", file.display()),
    }
    done();
    Ok(())
}

fn read(file: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

fn parse(file: &Path, text: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("{} isn't a JSON object", file.display())),
        Err(e) => Err(format!(
            "{} isn't valid JSON ({e}); fix it and run this again",
            file.display()
        )),
    }
}

fn is_ours(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(MARK) || c.contains(STOP_MARK) || c.contains(NUDGE_MARK))
}

/// The matchers whose groups run the guard.
fn installed(settings: &Map<String, Value>) -> Vec<String> {
    let groups = |event: &str| {
        settings
            .get("hooks")
            .and_then(|h| h.get(event))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let ours = |g: &Value| {
        g.get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hs| hs.iter().any(is_ours))
    };
    let mut out: Vec<String> = groups("PreToolUse")
        .iter()
        .filter(|g| ours(g))
        .map(|g| {
            g.get("matcher")
                .and_then(Value::as_str)
                .unwrap_or("*")
                .to_string()
        })
        .collect();
    if groups("PostToolUse").iter().any(ours) {
        out.push("PostToolUse".into());
    }
    if groups("Stop").iter().any(ours) {
        out.push("Stop".into());
    }
    out
}

fn pretty(settings: &Map<String, Value>) -> String {
    serde_json::to_string_pretty(settings).expect("settings serialize") + "\n"
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Agent {
    Claude,
    Codex,
}

/// Which hooks install writes beside the write stage on edits.
struct Stages {
    bash: bool,
    stop: bool,
    nudge: bool,
}

fn install(
    file: &Path,
    manifest: &Path,
    stages: Stages,
    runner: Option<&str>,
    agent: Agent,
) -> Result<(), String> {
    let Stages { bash, stop, nudge } = stages;
    let (edits, dir) = match agent {
        Agent::Claude => (EDIT_TOOLS, CLAUDE_DIR),
        Agent::Codex => (CODEX_EDIT_TOOLS, CODEX_DIR),
    };
    let original = read(file)?;
    let mut settings = match &original {
        Some(text) => parse(file, text)?,
        None => Map::new(),
    };
    if !installed(&settings).is_empty() {
        println!("hooks: already installed in {}", file.display());
        return Ok(());
    }
    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else {
        return Err(format!("{}: `hooks` isn't an object", file.display()));
    };
    let pre = hooks.entry("PreToolUse").or_insert_with(|| json!([]));
    let Some(pre) = pre.as_array_mut() else {
        return Err(format!(
            "{}: `hooks.PreToolUse` isn't a list",
            file.display()
        ));
    };
    let command = claude_command(runner, dir);
    let entry = json!({ "type": "command", "command": command, "timeout": TIMEOUT });
    pre.push(json!({ "matcher": edits, "hooks": [entry.clone()] }));
    if bash {
        pre.push(json!({ "matcher": "Bash", "hooks": [entry] }));
    }
    if stop {
        let groups = hooks.entry("Stop").or_insert_with(|| json!([]));
        let Some(groups) = groups.as_array_mut() else {
            return Err(format!("{}: `hooks.Stop` isn't a list", file.display()));
        };
        let command = stage_command("stop", runner, dir);
        groups.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": STOP_TIMEOUT }] }));
    }
    if nudge {
        let groups = hooks.entry("PostToolUse").or_insert_with(|| json!([]));
        let Some(groups) = groups.as_array_mut() else {
            return Err(format!(
                "{}: `hooks.PostToolUse` isn't a list",
                file.display()
            ));
        };
        let command = stage_command("nudge", runner, dir);
        groups.push(json!({ "matcher": edits, "hooks": [{ "type": "command", "command": command, "timeout": TIMEOUT }] }));
    }
    let written = pretty(&settings);
    let dir = file.parent().expect("a settings file has a directory");
    let made_dir = !dir.exists();
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // The first manifest is the one that knows the file before Fairlead.
    if !manifest.exists() {
        let record = Manifest {
            original,
            written: written.clone(),
            made_dir,
        };
        std::fs::create_dir_all(manifest.parent().expect("a manifest has a directory"))
            .and_then(|()| {
                std::fs::write(
                    manifest,
                    serde_json::to_string_pretty(&record).expect("manifest serializes"),
                )
            })
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
    }
    std::fs::write(file, &written).map_err(|e| format!("{}: {e}", file.display()))?;
    let what = match (bash, stop) {
        (true, true) => "edits and shell commands, and stops against `fairlead done`",
        (true, false) => "edits and shell commands",
        (false, true) => "edits, and stops against `fairlead done`",
        (false, false) => "edits",
    };
    let note = if nudge {
        "; it notes an edit made with no brief"
    } else {
        ""
    };
    println!(
        "hooks: installed in {}, checking {what}{note}",
        file.display()
    );
    if agent == Agent::Codex {
        println!("hooks: Codex runs a project's hooks once the project is trusted and you approve them; it asks when it starts, or see /hooks");
    }
    Ok(())
}

fn status(file: &Path, manifest: &Path) -> Result<(), String> {
    let Some(text) = read(file)? else {
        println!("hooks: not installed; {} doesn't exist", file.display());
        return Ok(());
    };
    let matchers = installed(&parse(file, &text)?);
    if matchers.is_empty() {
        println!("hooks: not installed in {}", file.display());
        return Ok(());
    }
    println!(
        "hooks: installed in {} for {}",
        file.display(),
        matchers.join(", ")
    );
    let exact = read_manifest(manifest)?.is_some_and(|m| m.written == text);
    if exact {
        println!("hooks: uninstall will restore the file byte for byte");
    } else {
        println!("hooks: the file changed since install, so uninstall will remove only Fairlead's entries");
    }
    Ok(())
}

fn read_manifest(manifest: &Path) -> Result<Option<Manifest>, String> {
    let Some(text) = read(manifest)? else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{}: {e}", manifest.display()))
}

fn uninstall(file: &Path, manifest_path: &Path) -> Result<(), String> {
    let current = read(file)?;
    let manifest = read_manifest(manifest_path)?;
    let remove_manifest = || {
        let _ = std::fs::remove_file(manifest_path);
    };
    if let (Some(text), Some(m)) = (&current, &manifest) {
        if *text == m.written {
            match &m.original {
                Some(original) => std::fs::write(file, original)
                    .map_err(|e| format!("{}: {e}", file.display()))?,
                None => {
                    std::fs::remove_file(file).map_err(|e| format!("{}: {e}", file.display()))?;
                    if m.made_dir {
                        // Only if nothing else went in since.
                        let _ = std::fs::remove_dir(
                            file.parent().expect("a settings file has a directory"),
                        );
                    }
                }
            }
            remove_manifest();
            println!(
                "hooks: uninstalled; {} is back as it was, byte for byte",
                file.display()
            );
            return Ok(());
        }
    }
    let Some(text) = current else {
        remove_manifest();
        println!("hooks: not installed; {} doesn't exist", file.display());
        return Ok(());
    };
    let mut settings = parse(file, &text)?;
    if installed(&settings).is_empty() {
        remove_manifest();
        println!("hooks: not installed in {}", file.display());
        return Ok(());
    }
    strip(&mut settings);
    std::fs::write(file, pretty(&settings)).map_err(|e| format!("{}: {e}", file.display()))?;
    remove_manifest();
    println!(
        "hooks: removed Fairlead's entries from {}; it changed since install, so the rest is kept but its formatting may differ",
        file.display()
    );
    Ok(())
}

/// Removes every guard entry, then any group, list or object it left empty.
fn strip(settings: &mut Map<String, Value>) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    for event in ["PreToolUse", "PostToolUse", "Stop"] {
        let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
            continue;
        };
        for group in groups.iter_mut() {
            if let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                list.retain(|h| !is_ours(h));
            }
        }
        groups.retain(|g| {
            g.get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|l| !l.is_empty())
        });
        if groups.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() {
        settings.remove("hooks");
    }
}
