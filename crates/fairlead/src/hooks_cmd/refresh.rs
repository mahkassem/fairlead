//! What `fairlead migrate` refreshes: hooks files whose Fairlead entries
//! aren't the ones this version's `hooks install` writes for the config.

use std::path::{Path, PathBuf};

use fairlead_guard::lefthook;
use serde_json::{Map, Value};

use super::{
    add_ours, claude_manifest, git_run, installed, is_ours, lefthook_file, lefthook_manifest,
    package_runner, parse, pretty, read, read_manifest, stages, strip, write, Agent, SESSION_EVENT,
};

/// One hooks file that isn't what this version would install, and what it would be.
pub struct Refresh {
    pub file: PathBuf,
    manifest: PathBuf,
    before: String,
    pub after: String,
    /// What changes, in a few words.
    pub what: String,
}

/// Fairlead's own entries, as (event, matcher, command, timeout), in order.
fn ours(settings: &Map<String, Value>) -> Vec<(String, String, String, u64)> {
    let mut out = Vec::new();
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else {
        return out;
    };
    for event in ["PreToolUse", "PostToolUse", "Stop", SESSION_EVENT] {
        for group in hooks
            .get(event)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let matcher = group.get("matcher").and_then(Value::as_str).unwrap_or("");
            for hook in group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if is_ours(hook) {
                    out.push((
                        event.to_string(),
                        matcher.to_string(),
                        hook.get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        hook.get("timeout").and_then(Value::as_u64).unwrap_or(0),
                    ));
                }
            }
        }
    }
    out
}

/// Which hooks are added or dropped between two sets of entries, or that
/// only their commands change.
fn difference(
    had: &[(String, String, String, u64)],
    want: &[(String, String, String, u64)],
) -> String {
    let name = |e: &(String, String, String, u64)| match e.0.as_str() {
        "PreToolUse" if e.1 == "Bash" => "the shell-command guard".to_string(),
        "PreToolUse" => "the edit guard".to_string(),
        "PostToolUse" => "the brief nudge".to_string(),
        SESSION_EVENT => "the session resume".to_string(),
        _ => "the Stop hook".to_string(),
    };
    let names = |v: &[(String, String, String, u64)]| v.iter().map(name).collect::<Vec<_>>();
    let (had_n, want_n) = (names(had), names(want));
    let added: Vec<_> = want_n
        .iter()
        .filter(|n| !had_n.contains(n))
        .cloned()
        .collect();
    let dropped: Vec<_> = had_n
        .iter()
        .filter(|n| !want_n.contains(n))
        .cloned()
        .collect();
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("adds {}", added.join(" and ")));
    }
    if !dropped.is_empty() {
        parts.push(format!("drops {}", dropped.join(" and ")));
    }
    if parts.is_empty() {
        parts.push("updates the hooks' commands".into());
    }
    parts.join(", ")
}

/// Each Claude Code settings file holding Fairlead's hooks that differ from
/// what this version's `hooks install` writes for this config.
pub fn stale_claude(
    root: &Path,
    git_dir: &Path,
    config: &fairlead_core::config::Config,
) -> Result<Vec<Refresh>, String> {
    let mut out = Vec::new();
    for name in ["settings.json", "settings.local.json"] {
        let file = root.join(".claude").join(name);
        let Some(text) = read(&file)? else { continue };
        let settings = parse(&file, &text)?;
        if installed(&settings).is_empty() {
            continue;
        }
        let mut want = settings.clone();
        strip(&mut want);
        add_ours(
            &mut want,
            &file,
            &stages(config),
            package_runner(root),
            Agent::Claude,
        )?;
        let (had, wanted) = (ours(&settings), ours(&want));
        if had == wanted {
            continue;
        }
        out.push(Refresh {
            what: difference(&had, &wanted),
            manifest: claude_manifest(git_dir, name),
            file,
            before: text,
            after: pretty(&want),
        });
    }
    Ok(out)
}

/// Whether either Claude Code settings file holds Fairlead's hooks.
pub fn claude_installed(root: &Path) -> bool {
    ["settings.json", "settings.local.json"].iter().any(|name| {
        let file = root.join(".claude").join(name);
        read(&file)
            .ok()
            .flatten()
            .and_then(|text| parse(&file, &text).ok())
            .is_some_and(|settings| !installed(&settings).is_empty())
    })
}

/// The lefthook config, when its commit stage runs another command than this version writes.
pub fn stale_git(root: &Path, git_dir: &Path) -> Result<Option<Refresh>, String> {
    let file = lefthook_file(root);
    let Some(text) = read(&file)? else {
        return Ok(None);
    };
    let want = git_run(package_runner(root));
    match lefthook::current_run(&text) {
        Some(run) if run != want => {
            let after = lefthook::set_run(&text, &want).expect("the stage line was found");
            Ok(Some(Refresh {
                what: format!("runs `{want}` instead of `{run}`"),
                manifest: lefthook_manifest(git_dir, &file),
                file,
                before: text,
                after,
            }))
        }
        _ => Ok(None),
    }
}

/// Writes the refreshed file, and moves uninstall's record along with it
/// when it still matched, so uninstall keeps restoring the file from before Fairlead.
pub fn apply(refresh: &Refresh) -> Result<(), String> {
    write(&refresh.file, &refresh.after)?;
    if let Some(mut record) = read_manifest(&refresh.manifest)? {
        if record.written == refresh.before {
            record.written = refresh.after.clone();
            write(
                &refresh.manifest,
                &serde_json::to_string_pretty(&record).expect("manifest serializes"),
            )?;
        }
    }
    Ok(())
}
