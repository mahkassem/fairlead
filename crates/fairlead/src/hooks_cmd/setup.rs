//! What `fairlead setup` asks of the hooks: the agents a repository shows,
//! the hooks each has, and installs that return what they did instead of
//! printing it.

use std::path::Path;

use fairlead_core::config::{Config, HooksTarget};

use super::{
    agent_file, git_install, install, installed, lefthook_file, lefthook_manifest, package_runner,
    parse, read, stages, wants_push, Agent,
};

/// Each agent's name as a person reads it, the folder that shows it is used
/// here, its hooks files, and the flag `hooks install` takes for it.
const AGENTS: [(Agent, &str, &str, &[&str], &str); 3] = [
    (
        Agent::Claude,
        "Claude Code",
        ".claude",
        &[".claude/settings.json", ".claude/settings.local.json"],
        "",
    ),
    (
        Agent::Codex,
        "Codex",
        ".codex",
        &[".codex/hooks.json"],
        " --codex",
    ),
    (
        Agent::Gemini,
        "Gemini CLI",
        ".gemini",
        &[".gemini/settings.json"],
        " --gemini",
    ),
];

/// One agent the repository shows, and the hooks it has.
pub struct AgentHooks {
    agent: Agent,
    pub name: &'static str,
    /// The command that installs its hooks.
    pub install: String,
    /// The stages installed, in words; empty when it has none.
    pub stages: Vec<&'static str>,
}

/// A stage as a person reads it, from the matcher or event `installed` names.
fn word(stage: &str) -> &'static str {
    match stage {
        "Bash" | "run_shell_command" => "shell guard",
        "PostToolUse" | "AfterTool" => "brief nudge",
        "Stop" | "AfterAgent" => "stop gate",
        "SessionStart" => "session start",
        _ => "edit guard",
    }
}

/// The agents this repository shows: Claude Code always, as the one
/// `hooks install` serves unasked, and each other whose folder is here.
pub fn agents(root: &Path) -> Vec<AgentHooks> {
    AGENTS
        .iter()
        .filter(|(agent, _, dir, _, _)| *agent == Agent::Claude || root.join(dir).is_dir())
        .map(|&(agent, name, _, files, flag)| {
            let mut stages: Vec<&'static str> = Vec::new();
            for rel in files {
                let file = root.join(rel);
                let found = read(&file)
                    .ok()
                    .flatten()
                    .and_then(|text| parse(&file, &text).ok())
                    .map(|settings| installed(&settings))
                    .unwrap_or_default();
                for stage in found.iter().map(|s| word(s)) {
                    if !stages.contains(&stage) {
                        stages.push(stage);
                    }
                }
            }
            AgentHooks {
                agent,
                name,
                install: format!("fairlead hooks install{flag}"),
                stages,
            }
        })
        .collect()
}

/// `hooks install` for each agent here that has no hooks, saying what each did.
pub fn install_agents(root: &Path, config: &Config) -> Result<Vec<String>, String> {
    let git_dir = fairlead_guard::git::git_dir(root).ok_or("not inside a git repository")?;
    let name = match config.hooks.claude {
        HooksTarget::Shared => "settings.json",
        HooksTarget::Local => "settings.local.json",
    };
    let mut said = Vec::new();
    for found in agents(root).iter().filter(|a| a.stages.is_empty()) {
        let (file, manifest) = agent_file(root, &git_dir, found.agent, name);
        said.extend(install(
            &file,
            &manifest,
            stages(config),
            package_runner(root),
            found.agent,
        )?);
    }
    Ok(said)
}

/// The git hooks through lefthook, as `hooks install --git` adds them,
/// returning what it would have printed.
pub fn install_git_quietly(root: &Path, config: &Config) -> Result<Vec<String>, String> {
    let git_dir = fairlead_guard::git::git_dir(root).ok_or("not inside a git repository")?;
    let lefthook = lefthook_file(root);
    let manifest = lefthook_manifest(&git_dir, &lefthook);
    git_install(
        &lefthook,
        &manifest,
        &git_dir,
        package_runner(root),
        wants_push(config),
    )
}
