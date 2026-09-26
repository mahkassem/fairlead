//! `fairlead hooks install | status | uninstall`: the Claude Code hook in
//! `.claude/settings.json` (shared) or `.claude/settings.local.json`. Install
//! keeps a manifest in the git directory with the file's original bytes and
//! the bytes it wrote, so uninstall can put the file back exactly.

use std::path::Path;
use std::process::ExitCode;

use clap::Subcommand;
use fairlead_core::config::{self, HooksTarget, LoadOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Runs the write stage, and does nothing where `fairlead` isn't installed,
/// so a teammate without it can still work.
const COMMAND: &str = "command -v fairlead >/dev/null 2>&1 && fairlead guard hook || true";
/// How an installed entry is recognised, whatever else is in the command.
const MARK: &str = "fairlead guard hook";
const EDIT_TOOLS: &str = "Edit|Write|MultiEdit";
/// Seconds; the hook keeps to its own much shorter budget.
const TIMEOUT: u64 = 10;

#[derive(Subcommand)]
pub enum HooksAction {
    /// Add the Claude Code hook.
    Install {
        #[command(flatten)]
        target: Target,
    },
    /// Say where the hook is and whether uninstall can restore the file exactly.
    Status {
        #[command(flatten)]
        target: Target,
    },
    /// Remove the hook, restoring the file byte for byte when nobody changed it since.
    Uninstall {
        #[command(flatten)]
        target: Target,
    },
}

#[derive(clap::Args)]
pub struct Target {
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
    let file = root.join(".claude").join(name);
    let manifest = git_dir
        .join("fairlead")
        .join("backups")
        .join(format!("claude-{name}"));
    let bash = !loaded.config.guard.commands.items().is_empty();
    let result = match action {
        HooksAction::Install { .. } => install(&file, &manifest, bash),
        HooksAction::Status { .. } => status(&file, &manifest),
        HooksAction::Uninstall { .. } => uninstall(&file, &manifest),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
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
        .is_some_and(|c| c.contains(MARK))
}

/// The matchers whose groups run the guard.
fn installed(settings: &Map<String, Value>) -> Vec<String> {
    let groups = settings
        .get("hooks")
        .and_then(|h| h.get("PreToolUse"))
        .and_then(Value::as_array);
    groups
        .into_iter()
        .flatten()
        .filter(|g| {
            g.get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|hs| hs.iter().any(is_ours))
        })
        .map(|g| {
            g.get("matcher")
                .and_then(Value::as_str)
                .unwrap_or("*")
                .to_string()
        })
        .collect()
}

fn pretty(settings: &Map<String, Value>) -> String {
    serde_json::to_string_pretty(settings).expect("settings serialize") + "\n"
}

fn install(file: &Path, manifest: &Path, bash: bool) -> Result<(), String> {
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
    let entry = json!({ "type": "command", "command": COMMAND, "timeout": TIMEOUT });
    pre.push(json!({ "matcher": EDIT_TOOLS, "hooks": [entry.clone()] }));
    if bash {
        pre.push(json!({ "matcher": "Bash", "hooks": [entry] }));
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
    let what = if bash {
        "edits and shell commands"
    } else {
        "edits"
    };
    println!("hooks: installed in {}, checking {what}", file.display());
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
    if let Some(pre) = hooks.get_mut("PreToolUse").and_then(Value::as_array_mut) {
        for group in pre.iter_mut() {
            if let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                list.retain(|h| !is_ours(h));
            }
        }
        pre.retain(|g| {
            g.get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|l| !l.is_empty())
        });
        if pre.is_empty() {
            hooks.remove("PreToolUse");
        }
    }
    if hooks.is_empty() {
        settings.remove("hooks");
    }
}
