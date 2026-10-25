//! `doctor`'s lines on which binary runs and which versions the repository
//! asks for, so a stale copy (an older `node_modules` linked into a new
//! worktree, or a global install ahead on PATH) shows before it fails a run.

use std::path::Path;

use serde_json::Value;

use crate::migrate_cmd::version;

/// The versions the repository names, read from disk.
#[derive(Debug, Default, PartialEq)]
pub struct Seen {
    /// The spec in package.json, such as `^0.12.0`.
    pub spec: Option<String>,
    /// The version of the package installed under `node_modules`.
    pub installed: Option<String>,
    /// The config's `fairlead` floor.
    pub floor: Option<String>,
}

pub fn read(root: &Path, floor: Option<&str>) -> Seen {
    let json = |rel: &str| -> Option<Value> {
        serde_json::from_str(&std::fs::read_to_string(root.join(rel)).ok()?).ok()
    };
    let spec = json("package.json").and_then(|m| {
        ["dependencies", "devDependencies", "optionalDependencies"]
            .iter()
            .find_map(|k| m.get(k)?.get("fairlead")?.as_str().map(String::from))
    });
    let installed = json("node_modules/fairlead/package.json")
        .and_then(|m| m.get("version")?.as_str().map(String::from));
    Seen {
        spec,
        installed,
        floor: floor.map(String::from),
    }
}

fn padded(text: &str) -> Option<[u64; 3]> {
    let v = version(text)?;
    Some([0, 1, 2].map(|i| v.get(i).copied().unwrap_or(0)))
}

/// The binary's path, the versions named, and a warning for each that
/// disagrees with the binary running.
pub fn lines(binary: &str, this: &str, seen: &Seen) -> String {
    let mut out = format!("binary: {binary}\n");
    let named: Vec<String> = [
        seen.spec.as_ref().map(|s| format!("package.json {s}")),
        seen.installed.as_ref().map(|s| format!("node_modules {s}")),
        seen.floor.as_ref().map(|s| format!("config floor {s}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !named.is_empty() {
        out.push_str(&format!("versions: {}\n", named.join(", ")));
    }
    let have = padded(this);
    if let (Some(spec), Some(have)) = (&seen.spec, have) {
        if padded(spec).is_some_and(|want| want > have) {
            out.push_str(&format!(
                "warning: package.json asks for {spec} but this binary is {this}; install again (a worktree's node_modules can be a copy of an older one)\n"
            ));
        }
    }
    if let Some(installed) = &seen.installed {
        if padded(installed) != have {
            out.push_str(&format!(
                "warning: node_modules has {installed} but this binary is {this}; run the project's copy through the package manager (`npx fairlead`, `bunx fairlead`) or install again\n"
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(spec: Option<&str>, installed: Option<&str>, floor: Option<&str>) -> Seen {
        Seen {
            spec: spec.map(String::from),
            installed: installed.map(String::from),
            floor: floor.map(String::from),
        }
    }

    #[test]
    fn matching_versions_name_the_binary_and_warn_of_nothing() {
        let out = lines(
            "/x/fairlead",
            "0.13.0",
            &seen(Some("^0.13.0"), Some("0.13.0"), Some("0.11")),
        );
        assert_eq!(
            out,
            "binary: /x/fairlead\nversions: package.json ^0.13.0, node_modules 0.13.0, config floor 0.11\n"
        );
    }

    #[test]
    fn a_binary_older_than_package_json_asks_warns() {
        let out = lines("/x/fairlead", "0.12.0", &seen(Some("^0.13.0"), None, None));
        assert!(
            out.contains("warning: package.json asks for ^0.13.0 but this binary is 0.12.0"),
            "{out}"
        );
    }

    #[test]
    fn a_binary_other_than_the_installed_package_warns_either_way() {
        for installed in ["0.11.0", "0.14.0"] {
            let out = lines(
                "/usr/bin/fairlead",
                "0.13.0",
                &seen(None, Some(installed), None),
            );
            assert!(
                out.contains(&format!("warning: node_modules has {installed}")),
                "{out}"
            );
        }
    }

    #[test]
    fn no_package_json_prints_only_the_binary() {
        assert_eq!(
            lines("/x/fairlead", "0.13.0", &Seen::default()),
            "binary: /x/fairlead\n"
        );
    }
}
