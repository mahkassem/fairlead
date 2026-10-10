//! Blueprints: named starting points `fairlead init --blueprint` writes once,
//! then the project owns. A blueprint adds config to what init detects, and
//! may write starter files and the CI workflow. A pack, by contrast, is a
//! layer every load reads; a blueprint can name one in `extends`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The built-in blueprints, by name.
pub const BUILTIN: &[(&str, &str)] = &[
    (
        "laravel-api",
        include_str!("../blueprints/laravel-api.toml"),
    ),
    ("vite-react", include_str!("../blueprints/vite-react.toml")),
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blueprint {
    pub name: String,
    pub description: String,
    /// Files whose presence suggests this blueprint; any one is enough.
    #[serde(default)]
    pub detect: Vec<String>,
    /// TOML added under what init detects.
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub files: Vec<File>,
    /// Also write `.github/workflows/fairlead.yml`, as `ci workflow --write` does.
    #[serde(default)]
    pub ci_workflow: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    /// From the repository root.
    pub path: String,
    pub text: String,
}

fn parse(text: &str, from: &str) -> Result<Blueprint, String> {
    let bp: Blueprint =
        toml::from_str(text).map_err(|e| format!("{from} isn't a blueprint: {e}"))?;
    for f in &bp.files {
        let p = Path::new(&f.path);
        if p.is_absolute()
            || p.components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "{from}: `{}` must be a plain path inside the repository",
                f.path
            ));
        }
    }
    Ok(bp)
}

/// A built-in blueprint's name, or a path to a blueprint file or to a
/// folder holding `blueprint.toml`.
pub fn load(name: &str, cwd: &Path) -> Result<Blueprint, String> {
    if let Some((_, text)) = BUILTIN.iter().find(|(n, _)| *n == name) {
        return parse(text, name);
    }
    let path = cwd.join(name);
    let file: PathBuf = if path.is_dir() {
        path.join("blueprint.toml")
    } else {
        path
    };
    match std::fs::read_to_string(&file) {
        Ok(text) => parse(&text, &file.display().to_string()),
        Err(_) => {
            let names: Vec<&str> = BUILTIN.iter().map(|(n, _)| *n).collect();
            Err(format!(
                "`{name}` is neither a built-in blueprint ({}) nor a blueprint file or folder",
                names.join(", ")
            ))
        }
    }
}

/// The built-in blueprint the repository looks like, if one.
pub fn suggest(files: &[String]) -> Option<Blueprint> {
    BUILTIN
        .iter()
        .filter_map(|(n, text)| parse(text, n).ok())
        .find(|bp| bp.detect.iter().any(|d| files.iter().any(|f| f == d)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_blueprint_parses_and_its_config_is_a_valid_layer() {
        for (name, text) in BUILTIN {
            let bp = parse(text, name).unwrap();
            assert_eq!(&bp.name, name);
            let layer: fairlead_core::config::Config =
                toml::from_str(&bp.config).unwrap_or_else(|e| panic!("{name}'s config: {e}"));
            assert!(
                fairlead_core::config::validate(&layer).is_empty(),
                "{name}: {:?}",
                fairlead_core::config::validate(&layer)
            );
            for f in &bp.files {
                assert!(
                    bp.config.contains(&f.path) || !f.path.ends_with("SKILL.md"),
                    "{name}: {} is routed",
                    f.path
                );
            }
        }
    }

    #[test]
    fn a_blueprint_file_cannot_write_outside_the_repository() {
        let text =
            "name = \"x\"\ndescription = \"x\"\n[[files]]\npath = \"../escape.sh\"\ntext = \"\"\n";
        assert!(parse(text, "x").unwrap_err().contains("plain path"));
    }
}
