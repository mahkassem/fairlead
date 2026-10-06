//! `fairlead hooks install | status | uninstall`: the Claude Code hook in
//! `.claude/settings.json` (shared) or `.claude/settings.local.json`, or the
//! same hooks for Codex in `.codex/hooks.json` and for Gemini CLI in
//! `.gemini/settings.json`. Install
//! keeps a manifest in the git directory with the file's original bytes and
//! the bytes it wrote, so uninstall can put the file back exactly.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use fairlead_core::config::{self, HooksTarget, LoadOptions};
use fairlead_guard::lefthook;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

mod refresh;
pub use refresh::{apply, claude_installed, stale_claude, stale_git, Refresh};

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
const RESUME_MARK: &str = "fairlead resume --hook";
/// Claude Code's event at the start of a session; installed for Claude Code only.
const SESSION_EVENT: &str = "SessionStart";
/// Seconds; the Stop hook hashes the working tree, which a large one makes slower.
const STOP_TIMEOUT: u64 = 30;
const EDIT_TOOLS: &str = "Edit|Write|MultiEdit";
/// Codex names its one editing tool; it also lets `Edit|Write` select it.
const CODEX_EDIT_TOOLS: &str = "apply_patch|Edit|Write";
const GEMINI_EDIT_TOOLS: &str = "write_file|replace";
/// Each agent's names for the three moments, before a tool, after it and at
/// the end of a turn; installed and stripped under any of them.
const EVENTS: [[&str; 3]; 2] = [
    ["PreToolUse", "PostToolUse", "Stop"],
    ["BeforeTool", "AfterTool", "AfterAgent"],
];
/// Where each agent's hook starts from: Claude Code says, and Codex starts a
/// hook in the session's directory, which can be below the root.
const CLAUDE_DIR: &str = "${CLAUDE_PROJECT_DIR:-.}";
const CODEX_DIR: &str = "$(git rev-parse --show-toplevel 2>/dev/null || pwd)";
/// Seconds; the hook keeps to its own much shorter budget.
const TIMEOUT: u64 = 10;

#[derive(Subcommand)]
pub enum HooksAction {
    /// Add the Claude Code hooks (the guard before an edit, the brief nudge
    /// after one, the Stop hook, `resume` when a session starts) and the git hook when lefthook is set up;
    /// `--codex` and `--gemini` add the same hooks for Codex or Gemini CLI.
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
    /// The same hooks for Gemini CLI, in `.gemini/settings.json`, instead of Claude Code's.
    #[arg(long, conflicts_with_all = ["claude", "git", "local", "shared", "codex"])]
    gemini: bool,
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
    } else if target.gemini {
        (
            root.join(".gemini").join("settings.json"),
            "gemini-settings.json".to_string(),
        )
    } else {
        (root.join(".claude").join(name), format!("claude-{name}"))
    };
    let manifest = git_dir.join("fairlead").join("backups").join(manifest_name);
    let lefthook = lefthook_file(&root);
    let lefthook_manifest = lefthook_manifest(&git_dir, &lefthook);
    let claude = !target.git;
    // Without `--git`, install only touches a lefthook config that's already there.
    let git = target.git
        || (!target.claude
            && !target.codex
            && !target.gemini
            && (lefthook.exists() || !matches!(action, HooksAction::Install { .. })));
    let agent = if target.codex {
        Agent::Codex
    } else if target.gemini {
        Agent::Gemini
    } else {
        Agent::Claude
    };
    let runner = package_runner(&root);
    let mut result = Ok(());
    if claude {
        result = match action {
            HooksAction::Install { .. } => {
                install(&file, &manifest, stages(&loaded.config), runner, agent)
            }
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
    // Only shown where another agent's hooks are, so a Claude Code project isn't told about them.
    for (agent, rel) in [
        ("codex", ".codex/hooks.json"),
        ("gemini", ".gemini/settings.json"),
    ] {
        let path = root.join(rel);
        if let Some(Ok(settings)) = read(&path).ok().flatten().map(|t| parse(&path, &t)) {
            if !installed(&settings).is_empty() {
                lines.push(format!(
                    "{agent} hook: installed in {rel} for {}",
                    installed(&settings).join(", ")
                ));
            }
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
fn stage_command(args: &str, runner: Option<&str>, dir: &str) -> String {
    match runner {
        None => {
            format!("command -v fairlead >/dev/null 2>&1 || exit 0; exec fairlead {args}")
        }
        Some(runner) => format!(
            "cd \"{dir}\" 2>/dev/null || exit 0; \
             if [ -x {NPM_BINARY} ]; then exec {NPM_BINARY} {args}; fi; \
             exec {runner} fairlead {args}"
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
    let run = git_run(runner);
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
        .is_some_and(|c| {
            [MARK, STOP_MARK, NUDGE_MARK, RESUME_MARK]
                .iter()
                .any(|m| c.contains(m))
        })
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
    let [pre, post, stop] = EVENTS
        .into_iter()
        .find(|events| events.iter().any(|e| !groups(e).is_empty()))
        .unwrap_or(EVENTS[0]);
    let mut out: Vec<String> = groups(pre)
        .iter()
        .filter(|g| ours(g))
        .map(|g| {
            g.get("matcher")
                .and_then(Value::as_str)
                .unwrap_or("*")
                .to_string()
        })
        .collect();
    if groups(post).iter().any(ours) {
        out.push(post.into());
    }
    if groups(stop).iter().any(ours) {
        out.push(stop.into());
    }
    if groups(SESSION_EVENT).iter().any(ours) {
        out.push(SESSION_EVENT.into());
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
    Gemini,
}

/// Which hooks install writes beside the write stage on edits.
struct Stages {
    bash: bool,
    stop: bool,
    nudge: bool,
    resume: bool,
}

/// The stages the config asks for.
fn stages(config: &fairlead_core::config::Config) -> Stages {
    Stages {
        bash: !config.guard.commands.items().is_empty(),
        stop: config.done.on_stop != fairlead_core::config::OnStop::Off,
        nudge: config.brief.nudge,
        resume: config.brief.resume,
    }
}

fn claude_manifest(git_dir: &Path, name: &str) -> PathBuf {
    git_dir
        .join("fairlead")
        .join("backups")
        .join(format!("claude-{name}"))
}

fn lefthook_manifest(git_dir: &Path, lefthook: &Path) -> PathBuf {
    let name = lefthook
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("lefthook.yml");
    git_dir
        .join("fairlead")
        .join("backups")
        .join(format!("lefthook-{name}"))
}

/// The commit stage's command, through the project's package runner when it has one.
fn git_run(runner: Option<&str>) -> String {
    match runner {
        Some(runner) => format!("{runner} {}", lefthook::RUN),
        None => lefthook::RUN.to_string(),
    }
}

/// Adds this version's entries for the stages to a settings map.
fn add_ours(
    settings: &mut Map<String, Value>,
    file: &Path,
    stages: &Stages,
    runner: Option<&str>,
    agent: Agent,
) -> Result<(), String> {
    let &Stages {
        bash,
        stop,
        nudge,
        resume,
    } = stages;
    let (edits, dir) = match agent {
        Agent::Claude => (EDIT_TOOLS, CLAUDE_DIR),
        Agent::Codex => (CODEX_EDIT_TOOLS, CODEX_DIR),
        Agent::Gemini => (GEMINI_EDIT_TOOLS, CODEX_DIR),
    };
    // Gemini CLI names the moments its own way and counts timeouts in milliseconds.
    let ([pre_event, post_event, stop_event], shell, unit) = match agent {
        Agent::Gemini => (EVENTS[1], "run_shell_command", 1000),
        _ => (EVENTS[0], "Bash", 1),
    };
    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));

    let Some(hooks) = hooks.as_object_mut() else {
        return Err(format!("{}: `hooks` isn't an object", file.display()));
    };
    let pre = hooks.entry(pre_event).or_insert_with(|| json!([]));
    let Some(pre) = pre.as_array_mut() else {
        return Err(format!(
            "{}: `hooks.{pre_event}` isn't a list",
            file.display()
        ));
    };
    let command = claude_command(runner, dir);
    let entry = json!({ "type": "command", "command": command, "timeout": TIMEOUT * unit });
    pre.push(json!({ "matcher": edits, "hooks": [entry.clone()] }));
    if bash {
        pre.push(json!({ "matcher": shell, "hooks": [entry] }));
    }
    if stop {
        let groups = hooks.entry(stop_event).or_insert_with(|| json!([]));
        let Some(groups) = groups.as_array_mut() else {
            return Err(format!(
                "{}: `hooks.{stop_event}` isn't a list",
                file.display()
            ));
        };
        let command = stage_command("guard stop", runner, dir);
        groups.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": STOP_TIMEOUT * unit }] }));
    }
    if nudge {
        let groups = hooks.entry(post_event).or_insert_with(|| json!([]));
        let Some(groups) = groups.as_array_mut() else {
            return Err(format!(
                "{}: `hooks.{post_event}` isn't a list",
                file.display()
            ));
        };
        let command = stage_command("guard nudge", runner, dir);
        // The same hook sees a skill load, so the hit rate can count it as used.
        let matcher = match agent {
            Agent::Claude => format!("{edits}|Skill"),
            Agent::Gemini => format!("{edits}|read_file"),
            Agent::Codex => edits.to_string(),
        };
        groups.push(json!({ "matcher": matcher, "hooks": [{ "type": "command", "command": command, "timeout": TIMEOUT * unit }] }));
    }
    if resume && agent == Agent::Claude {
        let groups = hooks.entry(SESSION_EVENT).or_insert_with(|| json!([]));
        let Some(groups) = groups.as_array_mut() else {
            return Err(format!(
                "{}: `hooks.{SESSION_EVENT}` isn't a list",
                file.display()
            ));
        };
        let command = stage_command("resume --hook", runner, dir);
        // Like the Stop hook, it plans the working tree, which a large one makes slower.
        groups.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": STOP_TIMEOUT }] }));
    }
    Ok(())
}

fn install(
    file: &Path,
    manifest: &Path,
    stages: Stages,
    runner: Option<&str>,
    agent: Agent,
) -> Result<(), String> {
    let Stages {
        bash, stop, nudge, ..
    } = stages;
    let original = read(file)?;
    let mut settings = match &original {
        Some(text) => parse(file, text)?,
        None => Map::new(),
    };
    if !installed(&settings).is_empty() {
        // migrate refreshes Claude Code's settings only.
        let hint = match agent {
            Agent::Claude => "; `fairlead migrate` brings them to this version's",
            _ => "",
        };
        println!("hooks: already installed in {}{hint}", file.display());
        return Ok(());
    }
    add_ours(&mut settings, file, &stages, runner, agent)?;
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
    let resume = if stages.resume && agent == Agent::Claude {
        "; a new session starts with `fairlead resume`"
    } else {
        ""
    };
    println!(
        "hooks: installed in {}, checking {what}{note}{resume}",
        file.display()
    );
    match agent {
        Agent::Codex => println!("hooks: Codex runs a project's hooks once the project is trusted and you approve them; it asks when it starts, or see /hooks"),
        Agent::Gemini => println!("hooks: Gemini CLI runs a project's hooks only in a trusted folder, and warns once when it first sees them"),
        Agent::Claude => {}
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
    for event in EVENTS.concat().into_iter().chain([SESSION_EVENT]) {
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
