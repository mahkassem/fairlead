//! `fairlead migrate` brings a repository to this binary's version: the
//! hooks it installed, the config's version floor and the version pins in
//! `package.json` and the workflows. Every step reads what the repository
//! has rather than which version it came from, so one run catches up from
//! any older release, and a second run finds nothing to do.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fairlead_core::config::{self, LoadOptions, Loaded};
use regex::Regex;
use serde_json::Value;

use crate::hooks_cmd::{self, Refresh};
use crate::migrate_notes::{Context, RELEASES, SINCE};

const SELF: &str = env!("CARGO_PKG_VERSION");
const ACTION: &str = "mahkassem/fairlead";

#[derive(clap::Args)]
pub struct MigrateArgs {
    /// Write the changes; without it, migrate only says what it would change.
    #[arg(long, conflicts_with = "check")]
    write: bool,
    /// Exit 1 when anything would change, so CI can catch a half-done upgrade.
    #[arg(long)]
    check: bool,
    /// The version the repository was on, which picks the release notes to
    /// show; read from its pins when left out.
    #[arg(long, value_name = "VERSION")]
    from: Option<String>,
}

enum Change {
    Text(String),
    Hooks(Refresh),
}

struct Update {
    file: PathBuf,
    what: String,
    change: Change,
}

/// A version as numbers, from `0.4`, `0.4.2`, `v0.4.2` or a package spec
/// such as `^0.4.2`; None for anything else.
pub fn version(text: &str) -> Option<Vec<u64>> {
    let text = text.trim().trim_start_matches(['^', '~', '=', 'v']);
    let text = text.strip_prefix(">=").unwrap_or(text);
    let parts: Option<Vec<u64>> = text.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| (1..=3).contains(&p.len()))
}

fn older(a: &[u64], b: &[u64]) -> bool {
    let pad = |v: &[u64]| [0, 1, 2].map(|i| v.get(i).copied().unwrap_or(0));
    pad(a) < pad(b)
}

fn this() -> Vec<u64> {
    version(SELF).expect("the crate version parses")
}

fn minor(v: &[u64]) -> String {
    format!("{}.{}", v[0], v.get(1).copied().unwrap_or(0))
}

/// The oldest version that reads every table the project file sets.
fn needed_floor(layer: &Value) -> Option<(Vec<u64>, &'static str)> {
    SINCE
        .iter()
        .filter(|(key, _)| {
            let pointer = format!("/{}", key.replace('.', "/"));
            layer.pointer(&pointer).is_some_and(|v| !v.is_null())
        })
        .map(|(key, since)| (version(since).expect("SINCE versions parse"), *key))
        .max_by(|a, b| a.0.cmp(&b.0))
}

/// The project file with its `fairlead` floor raised where it names a release
/// older than the newest table it uses.
fn floor(loaded: &Loaded) -> Result<Option<Update>, String> {
    let Some(file) = loaded.files.first() else {
        return Ok(None);
    };
    let layer = config::read_layer(file).map_err(|e| e.to_string())?;
    let Some(pin) = layer.get("fairlead").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(have) = version(pin) else {
        return Ok(None);
    };
    let Some((need, key)) = needed_floor(&layer) else {
        return Ok(None);
    };
    if !older(&have, &need) {
        return Ok(None);
    }
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let line = Regex::new(r#"(?m)^(fairlead\s*[=:]\s*)(["'])([^"']*)(["'])"#).expect("regex");
    let want = minor(&need);
    let Some(found) = line.captures(&text) else {
        return Ok(None);
    };
    let range = found.get(3).expect("group 3").range();
    let after = format!("{}{want}{}", &text[..range.start], &text[range.end..]);
    Ok(Some(Update {
        file: file.clone(),
        what: format!("`fairlead = \"{pin}\"` becomes \"{want}\": the config uses `{key}`, which {want} added"),
        change: Change::Text(after),
    }))
}

/// `package.json` with Fairlead's version spec moved to this release, and the
/// spec it had.
fn package_pin(root: &Path) -> Result<(Option<Update>, Option<String>), String> {
    let file = root.join("package.json");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Ok((None, None));
    };
    let Ok(manifest) = serde_json::from_str::<Value>(&text) else {
        return Ok((None, None));
    };
    let spec = ["dependencies", "devDependencies", "optionalDependencies"]
        .iter()
        .find_map(|k| manifest.get(k)?.get("fairlead")?.as_str().map(String::from));
    let Some(spec) = spec else {
        return Ok((None, None));
    };
    let Some(have) = version(&spec) else {
        return Ok((None, Some(spec)));
    };
    if !older(&have, &this()) || spec.contains(' ') {
        return Ok((None, Some(spec)));
    }
    let prefix: String = spec.chars().take_while(|c| !c.is_ascii_digit()).collect();
    let want = format!("{prefix}{SELF}");
    let pattern = Regex::new(&format!(
        r#""fairlead"(\s*):(\s*)"{}""#,
        regex::escape(&spec)
    ))
    .expect("regex");
    let after = pattern
        .replace_all(
            &text,
            format!(r#""fairlead"${{1}}:${{2}}"{want}""#).as_str(),
        )
        .into_owned();
    let update = (after != text).then(|| Update {
        file,
        what: format!(
            "fairlead `{spec}` becomes `{want}`; then {}",
            lock_hint(root)
        ),
        change: Change::Text(after),
    });
    Ok((update, Some(spec)))
}

/// The package manager command that brings the lockfile along.
fn lock_hint(root: &Path) -> &'static str {
    let has = |name: &str| root.join(name).is_file();
    if has("bun.lock") || has("bun.lockb") {
        "run `bun install` to update bun.lock"
    } else if has("pnpm-lock.yaml") {
        "run `pnpm install` to update pnpm-lock.yaml"
    } else if has("yarn.lock") {
        "run `yarn install` to update yarn.lock"
    } else if has("package-lock.json") {
        "run `npm install` to update package-lock.json"
    } else {
        "install it with your package manager"
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// A workflow with each step that uses Fairlead's action moved to this
/// release: the `@vX.Y.Z` ref and a `version:` input, where either names an
/// older release. Refs by branch or commit, and `latest`, are left alone.
fn workflow_pins(text: &str) -> (String, Vec<String>) {
    let uses = Regex::new(&format!(
        r#"^(\s*(?:-\s+)?uses:\s*["']?{}@)(v?)(\d+\.\d+\.\d+)"#,
        regex::escape(ACTION)
    ))
    .expect("regex");
    let input = Regex::new(r#"^(\s*version:\s*["']?)(v?)(\d+\.\d+\.\d+)"#).expect("regex");
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    let mut seen = Vec::new();
    let bump = |line: &mut String, re: &Regex, seen: &mut Vec<String>| {
        let Some(c) = re.captures(line) else { return };
        let old = c[3].to_string();
        if !version(&old).is_some_and(|v| older(&v, &this())) {
            return;
        }
        let range = c.get(3).expect("group 3").range();
        seen.push(format!("{}{old}", &c[2]));
        line.replace_range(range, SELF);
    };
    let marks: Vec<usize> = (0..lines.len())
        .filter(|&i| uses.is_match(&lines[i]))
        .collect();
    for at in marks {
        // The step runs from its `- ` line to the next line at that indent or less.
        let start = (0..=at)
            .rev()
            .find(|&i| {
                lines[i].trim_start().starts_with('-') && indent(&lines[i]) <= indent(&lines[at])
            })
            .unwrap_or(at);
        let dash = indent(&lines[start]);
        let end = (start + 1..lines.len())
            .find(|&i| !lines[i].trim().is_empty() && indent(&lines[i]) <= dash)
            .unwrap_or(lines.len());
        for line in &mut lines[start..end] {
            bump(line, &uses, &mut seen);
            bump(line, &input, &mut seen);
        }
    }
    (lines.join("\n"), seen)
}

/// Each workflow pinning an older release, with its text moved to this one
/// and the pins it had.
fn workflow_files(root: &Path) -> Vec<(PathBuf, String, Vec<String>)> {
    let dir = root.join(".github").join("workflows");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|file| {
            let text = std::fs::read_to_string(&file).ok()?;
            let (after, seen) = workflow_pins(&text);
            (!seen.is_empty()).then_some((file, after, seen))
        })
        .collect()
}

fn workflow_versions(root: &Path) -> Vec<(PathBuf, Vec<String>)> {
    workflow_files(root)
        .into_iter()
        .map(|(file, _, seen)| (file, seen))
        .collect()
}

fn workflows(root: &Path) -> Vec<Update> {
    workflow_files(root)
        .into_iter()
        .map(|(file, after, mut seen)| Update {
            what: {
                seen.dedup();
                format!("Fairlead's action at {} becomes v{SELF}", seen.join(", "))
            },
            file,
            change: Change::Text(after),
        })
        .collect()
}

/// Where the repository's version came from, for the release notes.
fn from_version(
    args_from: &Option<String>,
    package: &Option<String>,
    loaded: &Loaded,
    root: &Path,
) -> Option<(Vec<u64>, String)> {
    if let Some(v) = args_from {
        return version(v).map(|p| (p, format!("{v} (--from)")));
    }
    if let Some(spec) = package {
        if let Some(v) = version(spec) {
            return Some((v, format!("{spec} (package.json)")));
        }
    }
    for (file, seen) in workflow_versions(root) {
        if let Some(old) = seen.first() {
            let label = format!("{old} ({})", shown(root, &file));
            return version(old).map(|v| (v, label));
        }
    }
    let pin = loaded.config.fairlead.as_deref()?;
    version(pin).map(|v| (v, format!("{pin} (the config's floor)")))
}

fn shown(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

fn updates(root: &Path, loaded: &Loaded) -> Result<(Vec<Update>, Option<String>, bool), String> {
    let mut out = Vec::new();
    let mut claude = false;
    if let Some(git_dir) = fairlead_guard::git::git_dir(root) {
        for refresh in hooks_cmd::stale_claude(root, &git_dir, &loaded.config)? {
            out.push(Update {
                file: refresh.file.clone(),
                what: format!("Fairlead's hooks: {}", refresh.what),
                change: Change::Hooks(refresh),
            });
        }
        claude = hooks_cmd::claude_installed(root);
        if let Some(refresh) = hooks_cmd::stale_git(root, &git_dir)? {
            out.push(Update {
                file: refresh.file.clone(),
                what: format!("the commit stage {}", refresh.what),
                change: Change::Hooks(refresh),
            });
        }
    }
    out.extend(floor(loaded)?);
    let (package, spec) = package_pin(root)?;
    out.extend(package);
    out.extend(workflows(root));
    Ok((out, spec, claude))
}

fn notes(cx: &Context, from: Option<&[u64]>) -> Vec<(String, &'static str)> {
    RELEASES
        .iter()
        .filter_map(|(v, notes)| Some((*v, version(v)?, *notes)))
        .filter(|(_, v, _)| from.is_none_or(|f| older(f, v)) && !older(&this(), v))
        .flat_map(|(name, _, notes)| {
            notes
                .iter()
                .filter(|n| (n.applies)(cx))
                .map(move |n| (name.to_string(), n.text))
        })
        .collect()
}

fn write(update: &Update) -> Result<(), String> {
    match &update.change {
        Change::Text(text) => std::fs::write(&update.file, text)
            .map_err(|e| format!("{}: {e}", update.file.display())),
        Change::Hooks(refresh) => hooks_cmd::apply(refresh),
    }
}

pub fn run(args: MigrateArgs, cwd: &Path) -> ExitCode {
    let loaded = match config::load(cwd, &LoadOptions::from_process(Vec::new())) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("migrate: {e}; fix the config first, then run this again");
            return ExitCode::from(2);
        }
    };
    let root = crate::graph_cmd::repo_root(cwd);
    let (updates, spec, claude_hooks) = match updates(&root, &loaded) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("migrate: {e}");
            return ExitCode::from(2);
        }
    };
    let from = from_version(&args.from, &spec, &loaded, &root);
    match &from {
        Some((_, source)) => println!("migrate: from {source} to {SELF}"),
        None => println!(
            "migrate: to {SELF}; pass --from VERSION to see only the notes since that release"
        ),
    }
    let verb = if args.write {
        "updated"
    } else {
        "would update"
    };
    for update in &updates {
        println!("  {verb} {}: {}", shown(&root, &update.file), update.what);
    }
    let cx = Context {
        loaded: &loaded,
        root: &root,
        claude_hooks,
    };
    let review = notes(&cx, from.as_ref().map(|(v, _)| v.as_slice()));
    for (version, text) in &review {
        println!("  review ({version}): {text}");
    }
    for warning in &loaded.warnings {
        println!("  review (config): {}: {}", warning.key, warning.message);
    }
    if updates.is_empty() {
        println!("migrate: nothing to update");
        return ExitCode::SUCCESS;
    }
    if args.write {
        for update in &updates {
            if let Err(e) = write(update) {
                eprintln!("migrate: {e}");
                return ExitCode::from(2);
            }
        }
        println!(
            "migrate: {} file(s) updated; `fairlead doctor` checks the result",
            updates.len()
        );
        return ExitCode::SUCCESS;
    }
    println!(
        "migrate: dry run; `fairlead migrate --write` makes these {} change(s)",
        updates.len()
    );
    if args.check {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_read_from_specs_tags_and_floors() {
        assert_eq!(version("^0.4.2"), Some(vec![0, 4, 2]));
        assert_eq!(version("v0.6.0"), Some(vec![0, 6, 0]));
        assert_eq!(version("0.5"), Some(vec![0, 5]));
        assert_eq!(version(">=0.3.0"), Some(vec![0, 3, 0]));
        assert_eq!(version("latest"), None);
        assert_eq!(version("workspace:*"), None);
        assert!(older(&[0, 5], &[0, 5, 1]));
        assert!(!older(&[0, 6], &[0, 6, 0]));
    }

    #[test]
    fn every_release_in_the_changelog_has_an_entry() {
        let log =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md"))
                .expect("the changelog");
        let heading = Regex::new(r"(?m)^## (\d+\.\d+\.\d+)").expect("regex");
        let released: Vec<&str> = heading
            .captures_iter(&log)
            .map(|c| c.get(1).unwrap().as_str())
            .collect();
        let known: Vec<&str> = RELEASES.iter().map(|(v, _)| *v).collect();
        for v in &released {
            assert!(
                known.contains(v),
                "{v} is in CHANGELOG.md but has no entry in migrate_notes::RELEASES"
            );
        }
        let mut sorted = known.clone();
        sorted.sort_by_key(|v| version(v));
        assert_eq!(sorted, known, "RELEASES is oldest first");
    }

    #[test]
    fn every_newer_top_level_table_says_which_release_added_it() {
        let schema = config::json_schema();
        let tables = schema["properties"].as_object().expect("properties");
        for key in tables.keys() {
            let dated = SINCE.iter().any(|(k, _)| k == key);
            assert!(
                crate::migrate_notes::BASE.contains(&key.as_str()) || dated,
                "`{key}` needs an entry in migrate_notes::SINCE"
            );
        }
    }

    #[test]
    fn the_action_ref_and_its_version_input_move_and_nothing_else_does() {
        let text = "jobs:\n  plan:\n    steps:\n      - uses: actions/checkout@v4\n        with:\n          version: 0.1.0\n      - uses: mahkassem/fairlead@v0.4.2\n        with:\n          version: v0.4.2\n          command: ci plan\n      - uses: mahkassem/fairlead@main\n        with:\n          version: latest\n";
        let (after, seen) = workflow_pins(text);
        assert_eq!(seen, vec!["v0.4.2", "v0.4.2"]);
        assert!(after.contains(&format!("mahkassem/fairlead@v{SELF}")));
        assert!(after.contains(&format!("version: v{SELF}")));
        assert!(after.contains("actions/checkout@v4\n        with:\n          version: 0.1.0"));
        assert!(after.contains("mahkassem/fairlead@main"));
        let (again, none) = workflow_pins(&after);
        assert!(none.is_empty());
        assert_eq!(again, after);
    }
}
