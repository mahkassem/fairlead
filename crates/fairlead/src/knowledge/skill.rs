//! Skills in the open SKILL.md format: Fairlead reads only their name and
//! description, and takes their scope from `[[skills.routes]]`, so the file
//! itself stays one every agent reads untouched.

use std::path::Path;

use fairlead_core::config::{SkillRoute, Skills};
use serde::Deserialize;

use super::lesson::Bad;
use super::route::Scope;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The SKILL.md, from the repository root.
    pub path: String,
    pub scope: Scope,
}

#[derive(Deserialize, Default)]
struct Front {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// The name and description a SKILL.md gives, or its directory's name and
/// an empty description when its front matter doesn't say.
fn read(root: &Path, route: &SkillRoute) -> Result<(String, String), String> {
    let text = std::fs::read_to_string(root.join(&route.skill))
        .map_err(|e| format!("can't read it: {e}"))?;
    let front = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .and_then(|rest| rest.find("\n---").map(|end| &rest[..end + 1]))
        .map(|yaml| serde_saphyr::from_str::<Front>(yaml).map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or_default();
    let dir_name = Path::new(&route.skill)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("skill")
        .to_string();
    Ok((
        front
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(dir_name),
        front
            .description
            .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default(),
    ))
}

/// Every routed skill, and the routes whose skill can't be read.
pub fn load(root: &Path, skills: &Skills) -> (Vec<Skill>, Vec<Bad>) {
    let mut out = Vec::new();
    let mut bad = Vec::new();
    for route in skills.routes.items() {
        let scope = Scope::new(&route.paths, &route.modules, route.always);
        match (read(root, route), scope) {
            (Ok((name, description)), Ok(scope)) => out.push(Skill {
                name,
                description,
                path: route.skill.clone(),
                scope,
            }),
            (Err(reason), _) | (_, Err(reason)) => bad.push(Bad {
                path: route.skill.clone(),
                reason,
            }),
        }
    }
    (out, bad)
}

/// The routed skill a path names: the SKILL.md itself, or its directory.
pub fn by_path<'a>(skills: &'a [Skill], path: &str) -> Option<&'a Skill> {
    let path = path.trim_start_matches("./");
    skills.iter().find(|s| {
        path == s.path
            || path.ends_with(&format!("/{}", s.path))
            || Path::new(&s.path).parent().is_some_and(|d| {
                !d.as_os_str().is_empty() && path.starts_with(&format!("{}/", d.display()))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fairlead_core::config::List;

    fn route(skill: &str, paths: &[&str]) -> SkillRoute {
        SkillRoute {
            skill: skill.into(),
            paths: paths.iter().map(|s| s.to_string()).collect(),
            modules: Vec::new(),
            always: false,
        }
    }

    #[test]
    fn name_and_description_come_from_front_matter_or_the_directory() {
        let dir = std::env::temp_dir().join(format!("fairlead-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claude/skills/forms")).unwrap();
        std::fs::create_dir_all(dir.join(".claude/skills/bare")).unwrap();
        std::fs::write(
            dir.join(".claude/skills/forms/SKILL.md"),
            "---\nname: web-forms\ndescription: >\n  How forms\n  validate.\nallowed-tools: Read\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(dir.join(".claude/skills/bare/SKILL.md"), "Just a body\n").unwrap();
        let skills = Skills {
            routes: List::Items(vec![
                route(".claude/skills/forms/SKILL.md", &["src/forms/**"]),
                route(".claude/skills/bare/SKILL.md", &["src/**"]),
                route(".claude/skills/gone/SKILL.md", &["src/**"]),
            ]),
            ..Skills::default()
        };
        let (found, bad) = load(&dir, &skills);
        assert_eq!(found[0].name, "web-forms");
        assert_eq!(found[0].description, "How forms validate.");
        assert_eq!(found[1].name, "bare");
        assert_eq!(bad.len(), 1);
        assert!(bad[0].reason.contains("can't read it"), "{bad:?}");
        assert_eq!(
            by_path(&found, ".claude/skills/forms/SKILL.md").map(|s| s.name.as_str()),
            Some("web-forms")
        );
        assert_eq!(
            by_path(&found, "/abs/repo/.claude/skills/bare/SKILL.md").map(|s| s.name.as_str()),
            Some("bare")
        );
        assert!(by_path(&found, "src/forms/a.ts").is_none());
    }
}
