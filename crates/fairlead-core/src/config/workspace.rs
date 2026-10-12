//! A workspace: a folder holding several repositories side by side. Its
//! config file declares `[workspace]`, and its other settings are a layer
//! under each repository's own, read only outside CI, since a CI checkout
//! is one repository and plans itself.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::load::{error, find_config, read_layer, ConfigError};

/// A workspace file and the repositories it holds.
#[derive(Debug, Clone)]
pub struct Found {
    pub file: PathBuf,
    pub root: PathBuf,
    pub repos: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub name: String,
    /// From the workspace folder, with `/`.
    pub path: String,
    pub dir: PathBuf,
    pub base: Option<String>,
}

impl Found {
    pub fn names(&self) -> String {
        let names: Vec<&str> = self.repos.iter().map(|m| m.name.as_str()).collect();
        names.join(", ")
    }

    /// The repository named `name`, or an error naming the ones there are.
    pub fn member(&self, name: &str) -> Result<&Member, String> {
        self.repos.iter().find(|m| m.name == name).ok_or_else(|| {
            format!(
                "no repository `{name}` in the workspace {}: {}",
                self.file.display(),
                self.names()
            )
        })
    }

    /// The repository holding `path`, an absolute path.
    pub fn holding(&self, path: &Path) -> Option<&Member> {
        self.repos.iter().find(|m| path.starts_with(&m.dir))
    }
}

/// The repository root at or above `start`: the first directory with `.git`.
pub fn git_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The workspace `file` declares, or none when it declares no `[workspace]`.
pub fn read(file: &Path) -> Result<Option<Found>, ConfigError> {
    let value = read_layer(file)?;
    let Some(table) = value.get("workspace") else {
        return Ok(None);
    };
    let label = file.display().to_string();
    let root = canonical(file.parent().unwrap_or(Path::new(".")));
    let listed = table.get("repos").and_then(Value::as_array);
    let mut repos = Vec::new();
    match listed.filter(|l| !l.is_empty()) {
        Some(list) => {
            for entry in list {
                let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
                let dir = canonical(&root.join(path));
                if path.is_empty() || !dir.join(".git").exists() || !dir.starts_with(&root) {
                    return Err(error(
                        &label,
                        Some("workspace.repos".into()),
                        format!("`{path}` isn't a git repository inside the workspace folder"),
                    ));
                }
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| last_segment(path));
                let base = entry
                    .get("base")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                repos.push(Member {
                    name,
                    path: path.trim_end_matches('/').replace('\\', "/"),
                    dir,
                    base,
                });
            }
        }
        None => {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
                .map_err(|e| error(&label, None, e.to_string()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.join(".git").exists())
                .collect();
            dirs.sort();
            for dir in dirs {
                let name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                repos.push(Member {
                    path: name.clone(),
                    name,
                    dir: canonical(&dir),
                    base: None,
                });
            }
        }
    }
    for (i, m) in repos.iter().enumerate() {
        if repos[..i].iter().any(|o| o.name == m.name) {
            return Err(error(
                &label,
                Some("workspace.repos".into()),
                format!("two repositories are named `{}`; give one a `name`", m.name),
            ));
        }
    }
    Ok(Some(Found {
        file: file.to_path_buf(),
        root,
        repos,
    }))
}

fn last_segment(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_string()
}

/// The workspace whose folder is at or above `start`, when `start` is in no
/// repository.
pub fn at(start: &Path) -> Result<Option<Found>, ConfigError> {
    if git_root(start).is_some() {
        return Ok(None);
    }
    match find_config(start)? {
        Some(file) => read(&file),
        None => Ok(None),
    }
}

/// The workspace that holds the repository at `repo`: the nearest folder
/// above it whose workspace file lists the repository or, listing none,
/// has it one level down.
pub fn enclosing(repo: &Path) -> Result<Option<Found>, ConfigError> {
    let repo = canonical(repo);
    for dir in repo.ancestors().skip(1) {
        let Some(file) = super::load::PROJECT_NAMES
            .iter()
            .map(|n| dir.join(n))
            .find(|p| p.is_file())
        else {
            continue;
        };
        if let Some(found) = read(&file)? {
            if found.repos.iter().any(|m| m.dir == repo) {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}
