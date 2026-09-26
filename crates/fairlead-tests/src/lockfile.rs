//! Which workspace packages a pnpm lockfile change can reach. A package
//! (importer) is affected when its own entry changed or it depends, in the
//! base or the head lockfile, on a package whose entry changed. Anything
//! this can't read with certainty answers `None`, and the planner runs
//! everything, as for any other run-all path.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

/// Top-level keys that may differ: the ones this module follows. `catalogs`
/// is derived data, since every consumer's importer entry carries the
/// version its `catalog:` specifier resolved to.
const FOLLOWED: [&str; 4] = ["importers", "packages", "snapshots", "catalogs"];
const DEP_FIELDS: [&str; 3] = ["dependencies", "devDependencies", "optionalDependencies"];

/// The importers (workspace package paths, `.` for the root) the change
/// from `base` to `head` can affect.
pub fn affected_importers(base: &str, head: &str) -> Option<BTreeSet<String>> {
    let base = Lock::parse(base)?;
    let head = Lock::parse(head)?;
    let keys: BTreeSet<&String> = base.top.keys().chain(head.top.keys()).collect();
    if keys
        .iter()
        .any(|k| !FOLLOWED.contains(&k.as_str()) && base.top.get(*k) != head.top.get(*k))
    {
        return None;
    }
    let changed_nodes = changed(&base.nodes, &head.nodes);
    let mut affected: BTreeSet<String> = changed(&base.importers, &head.importers);
    for lock in [&base, &head] {
        affected.extend(lock.reaching(&changed_nodes)?);
    }
    // Node resolution walks up, so a package nested in an affected one sees
    // its dependencies too.
    let all: BTreeSet<&String> = base.importers.keys().chain(head.importers.keys()).collect();
    let nested: Vec<String> = all
        .iter()
        .filter(|inner| {
            affected.iter().any(|outer| {
                outer != **inner && (outer == "." || inner.starts_with(&format!("{outer}/")))
            })
        })
        .map(|s| s.to_string())
        .collect();
    affected.extend(nested);
    Some(affected)
}

/// Keys present on one side only, or whose values differ.
fn changed(base: &BTreeMap<String, Value>, head: &BTreeMap<String, Value>) -> BTreeSet<String> {
    let keys: BTreeSet<&String> = base.keys().chain(head.keys()).collect();
    keys.into_iter()
        .filter(|k| base.get(*k) != head.get(*k))
        .cloned()
        .collect()
}

struct Lock {
    top: Map<String, Value>,
    importers: BTreeMap<String, Value>,
    /// Resolved packages by key, with a v9 `packages` entry folded into
    /// every snapshot of it so a changed resolution changes the snapshot.
    nodes: BTreeMap<String, Value>,
}

impl Lock {
    fn parse(text: &str) -> Option<Lock> {
        let Value::Object(top) = serde_saphyr::from_str::<Value>(text).ok()? else {
            return None;
        };
        let version = match top.get("lockfileVersion")? {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };
        let major: u32 = version.split('.').next()?.parse().ok()?;
        if !(6..=9).contains(&major) {
            return None;
        }
        let object = |key: &str| -> Option<BTreeMap<String, Value>> {
            match top.get(key) {
                None | Some(Value::Null) => Some(BTreeMap::new()),
                Some(Value::Object(m)) => Some(m.clone().into_iter().collect()),
                Some(_) => None,
            }
        };
        let importers = object("importers")?;
        let packages: BTreeMap<String, Value> = object("packages")?
            .into_iter()
            .map(|(k, v)| (k.trim_start_matches('/').to_string(), v))
            .collect();
        let nodes = if major >= 9 {
            object("snapshots")?
                .into_iter()
                .map(|(key, snapshot)| {
                    let package = packages.get(peerless(&key)).cloned().unwrap_or(Value::Null);
                    (key, Value::Array(vec![snapshot, package]))
                })
                .collect()
        } else {
            packages
        };
        Some(Lock {
            top,
            importers,
            nodes,
        })
    }

    /// The node a dependency entry names, `None` when it names nothing
    /// known, and `Some(None)` for a workspace link the source graph covers.
    fn resolve(&self, name: &str, version: &str) -> Option<Option<String>> {
        if version.starts_with("link:") {
            return Some(None);
        }
        let version = version.trim_start_matches('/');
        [format!("{name}@{version}"), version.to_string()]
            .into_iter()
            .find(|key| self.nodes.contains_key(key))
            .map(Some)
    }

    /// The nodes each node and importer depends on; `None` if any entry
    /// names a node the lockfile doesn't have.
    fn edges(&self) -> Option<BTreeMap<String, Vec<String>>> {
        let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (importer, entry) in &self.importers {
            let mut out = Vec::new();
            for (name, dep) in deps(entry) {
                let version = match dep {
                    Value::Object(d) => d.get("version").and_then(Value::as_str)?.to_string(),
                    Value::String(v) => v.clone(),
                    _ => return None,
                };
                if let Some(key) = self.resolve(&name, &version)? {
                    out.push(key);
                }
            }
            edges.insert(format!("importer:{importer}"), out);
        }
        for (key, node) in &self.nodes {
            let snapshot = match node {
                Value::Array(parts) => parts.first().cloned().unwrap_or(Value::Null),
                other => other.clone(),
            };
            let mut out = Vec::new();
            for (name, dep) in deps(&snapshot) {
                let version = dep.as_str()?.to_string();
                if let Some(target) = self.resolve(&name, &version)? {
                    out.push(target);
                }
            }
            edges.insert(key.clone(), out);
        }
        Some(edges)
    }

    /// Importers from which a changed node is reachable.
    fn reaching(&self, changed: &BTreeSet<String>) -> Option<BTreeSet<String>> {
        let edges = self.edges()?;
        let mut reverse: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (from, tos) in &edges {
            for to in tos {
                reverse.entry(to.as_str()).or_default().push(from.as_str());
            }
        }
        let mut seen: BTreeSet<&str> = changed.iter().map(String::as_str).collect();
        let mut queue: Vec<&str> = seen.iter().copied().collect();
        while let Some(node) = queue.pop() {
            for &from in reverse.get(node).into_iter().flatten() {
                if seen.insert(from) {
                    queue.push(from);
                }
            }
        }
        Some(
            seen.into_iter()
                .filter_map(|n| n.strip_prefix("importer:").map(str::to_string))
                .collect(),
        )
    }
}

/// `name@1.0.0(peer@2.0.0)` without its peer suffix.
fn peerless(key: &str) -> &str {
    let start = usize::from(key.starts_with('@'));
    match key[start..].find('(') {
        Some(i) => &key[..start + i],
        None => key,
    }
}

fn deps(entry: &Value) -> Vec<(String, Value)> {
    DEP_FIELDS
        .iter()
        .filter_map(|field| entry.get(*field).and_then(Value::as_object))
        .flat_map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peer_suffix_is_dropped_from_a_key() {
        assert_eq!(peerless("vite@8.0.1(@types/node@24.1.0)"), "vite@8.0.1");
        assert_eq!(peerless("@scope/a@1.0.0(b@2.0.0)"), "@scope/a@1.0.0");
        assert_eq!(peerless("left-pad@1.3.0"), "left-pad@1.3.0");
    }
}
