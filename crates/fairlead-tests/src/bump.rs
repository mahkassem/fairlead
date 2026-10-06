//! Whether a `package.json` or `bun.lock` change only moves dependency
//! versions, and which packages it moves. Anything more, or anything this
//! can't read, answers `None`, and the planner treats the file as before.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

const DEP_FIELDS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

/// The dependencies whose version strings moved between two `package.json`
/// texts. The package's own `version` may move too, and moves nothing.
pub fn manifest_bumps(base: &str, head: &str) -> Option<BTreeSet<String>> {
    let base: Value = serde_json::from_str(base).ok()?;
    let head: Value = serde_json::from_str(head).ok()?;
    entry_bumps(base.as_object()?, head.as_object()?)
}

/// A manifest, or a lockfile's copy of one: dependency fields may differ
/// only in version strings, `version` freely, and nothing else at all.
fn entry_bumps(base: &Map<String, Value>, head: &Map<String, Value>) -> Option<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let keys: BTreeSet<&String> = base.keys().chain(head.keys()).collect();
    for key in keys {
        let (b, h) = (base.get(key), head.get(key));
        if b == h {
            continue;
        }
        if key == "version" && b.is_some_and(Value::is_string) && h.is_some_and(Value::is_string) {
            continue;
        }
        if !DEP_FIELDS.contains(&key.as_str()) {
            return None;
        }
        out.extend(moved(b?.as_object()?, h?.as_object()?)?);
    }
    Some(out)
}

/// The names whose version string moved, when both sides name the same
/// packages.
fn moved(base: &Map<String, Value>, head: &Map<String, Value>) -> Option<Vec<String>> {
    if base.len() != head.len() || base.keys().any(|k| !head.contains_key(k)) {
        return None;
    }
    let mut out = Vec::new();
    for (name, b) in base {
        let h = &head[name];
        if b == h {
            continue;
        }
        if !(is_version(b.as_str()?) && is_version(h.as_str()?)) {
            return None;
        }
        out.push(name.clone());
    }
    Some(out)
}

/// A registry version or range. A protocol, path, alias or URL can point
/// somewhere else entirely, which is more than a version moving.
fn is_version(spec: &str) -> bool {
    !spec.trim().is_empty() && !spec.contains(':') && !spec.contains('/')
}

fn is_integrity(text: &str) -> bool {
    ["sha512-", "sha384-", "sha256-", "sha1-"]
        .iter()
        .any(|p| text.starts_with(p))
}

/// A parsed text `bun.lock` (lockfile versions 0 and 1).
pub struct BunLock {
    top: Map<String, Value>,
}

impl BunLock {
    pub fn parse(text: &str) -> Option<BunLock> {
        let value: Value = serde_json::from_str(&without_trailing_commas(text)).ok()?;
        let mut top = value.as_object()?.clone();
        if top.get("lockfileVersion")?.as_u64()? > 1 {
            return None;
        }
        for key in ["workspaces", "packages"] {
            if !top
                .entry(key)
                .or_insert_with(|| Value::Object(Map::new()))
                .is_object()
            {
                return None;
            }
        }
        Some(BunLock { top })
    }

    fn object(&self, key: &str) -> &Map<String, Value> {
        self.top[key].as_object().expect("checked when parsed")
    }

    /// Each package's installed name with the names its entry depends on.
    fn edges(&self) -> impl Iterator<Item = (&str, &str)> {
        self.object("packages").iter().flat_map(|(key, entry)| {
            let meta = entry
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_object);
            meta.flat_map(|m| {
                DEP_FIELDS
                    .iter()
                    .filter_map(|f| m.get(*f).and_then(Value::as_object))
                    .flat_map(|deps| deps.keys())
            })
            .map(move |dep| (installed(key), dep.as_str()))
        })
    }

    /// The commands a package installs, from its entry's `bin`.
    pub fn bins(&self, name: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for (key, entry) in self.object("packages") {
            if installed(key) != name {
                continue;
            }
            for meta in entry.as_array().into_iter().flatten() {
                match meta.get("bin") {
                    Some(Value::Object(bins)) => out.extend(bins.keys().cloned()),
                    Some(Value::String(_)) => {
                        out.insert(name.rsplit('/').next().unwrap_or(name).to_string());
                    }
                    _ => {}
                }
            }
        }
        out
    }
}

/// The packages whose versions moved from `base` to `head`, by installed
/// name, with any workspace's moved dependency ranges.
pub fn lock_bumps(base: &BunLock, head: &BunLock) -> Option<BTreeSet<String>> {
    let keys: BTreeSet<&String> = base.top.keys().chain(head.top.keys()).collect();
    if keys.iter().any(|k| {
        !["workspaces", "packages"].contains(&k.as_str()) && base.top.get(*k) != head.top.get(*k)
    }) {
        return None;
    }
    let mut out = BTreeSet::new();
    let (bw, hw) = (base.object("workspaces"), head.object("workspaces"));
    for (dir, b) in same_keys(bw, hw)? {
        out.extend(entry_bumps(b.as_object()?, hw[dir].as_object()?)?);
    }
    let (bp, hp) = (base.object("packages"), head.object("packages"));
    for (key, b) in same_keys(bp, hp)? {
        package_moved(b, &hp[key])?;
        out.insert(installed(key).to_string());
    }
    Some(out)
}

/// The base's entries that differ from the head's, when both have the same
/// keys: a package added or removed is more than a version moving.
fn same_keys<'a>(
    base: &'a Map<String, Value>,
    head: &'a Map<String, Value>,
) -> Option<Vec<(&'a String, &'a Value)>> {
    if base.len() != head.len() || base.keys().any(|k| !head.contains_key(k)) {
        return None;
    }
    Some(base.iter().filter(|(k, v)| head[*k] != **v).collect())
}

/// A package entry, `["name@1.0.0", registry, {meta}, integrity]`, whose
/// resolved version, integrity and dependency versions alone moved.
fn package_moved(base: &Value, head: &Value) -> Option<()> {
    let (b, h) = (base.as_array()?, head.as_array()?);
    if b.len() != h.len() || b.is_empty() {
        return None;
    }
    let (b_name, b_version) = resolution(b[0].as_str()?)?;
    let (h_name, h_version) = resolution(h[0].as_str()?)?;
    if b_name != h_name || !is_version(b_version) || !is_version(h_version) {
        return None;
    }
    let last = b.len() - 1;
    for (i, (x, y)) in b.iter().zip(h).enumerate().skip(1) {
        match (x, y) {
            _ if x == y => {}
            (Value::Object(x), Value::Object(y)) => {
                entry_bumps(x, y)?;
            }
            (Value::String(x), Value::String(y))
                if i == last && is_integrity(x) && is_integrity(y) => {}
            _ => return None,
        }
    }
    Some(())
}

/// `@scope/name@1.0.0` as the name and the version.
fn resolution(text: &str) -> Option<(&str, &str)> {
    let at = text.get(1..)?.rfind('@')? + 1;
    Some((&text[..at], &text[at + 1..]))
}

/// The name a lockfile key installs under: its last segment, `parent/@s/x`
/// being `@s/x`, nested inside the package that needs that version.
pub fn installed(key: &str) -> &str {
    let mut parts = key.rsplitn(3, '/');
    let last = parts.next().unwrap_or(key);
    match parts.next() {
        Some(scope) if scope.starts_with('@') => &key[key.len() - scope.len() - 1 - last.len()..],
        _ => last,
    }
}

/// `names`, each with what named it, plus every package in either lock that
/// depends on one of them, directly or not, carrying the same label: a
/// package that loads a moved one behaves differently too.
pub fn dependents(locks: &[&BunLock], names: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut reverse: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for lock in locks {
        for (from, to) in lock.edges() {
            reverse.entry(to).or_default().insert(from);
        }
    }
    let mut out = names;
    let mut queue: Vec<(String, String)> = out.clone().into_iter().collect();
    while let Some((name, label)) = queue.pop() {
        for from in reverse.get(name.as_str()).into_iter().flatten() {
            if !out.contains_key(*from) {
                out.insert(from.to_string(), label.clone());
                queue.push((from.to_string(), label.clone()));
            }
        }
    }
    out
}

/// `bun.lock` is JSON with trailing commas, which `serde_json` refuses.
fn without_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    for (i, c) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        } else if c == ',' && matches!(text[i + 1..].trim_start().chars().next(), Some('}' | ']')) {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
      "name": "app",
      "version": "1.0.0",
      "scripts": { "test": "bun test" },
      "dependencies": { "left": "^1.0.0" },
      "devDependencies": { "tool": "1.0.0", "@s/kit": "~2.0.0" }
    }"#;

    fn set(names: &[&str]) -> Option<BTreeSet<String>> {
        Some(names.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn a_manifest_whose_dependency_versions_moved_names_them() {
        let head = MANIFEST
            .replace(r#""tool": "1.0.0""#, r#""tool": "1.0.1""#)
            .replace("~2.0.0", "~2.1.0");
        assert_eq!(manifest_bumps(MANIFEST, &head), set(&["@s/kit", "tool"]));
    }

    #[test]
    fn a_manifest_whose_own_version_moved_moves_no_package() {
        let head = MANIFEST.replace(r#""version": "1.0.0""#, r#""version": "1.1.0""#);
        assert_eq!(manifest_bumps(MANIFEST, &head), set(&[]));
    }

    #[test]
    fn a_manifest_change_beyond_versions_is_not_a_bump() {
        let cases = [
            MANIFEST.replace("bun test", "bun test --bail"),
            MANIFEST.replace(
                r#""left": "^1.0.0""#,
                r#""left": "^1.0.0", "right": "1.0.0""#,
            ),
            MANIFEST.replace(r#""left": "^1.0.0""#, r#""right": "^1.0.0""#),
            MANIFEST.replace("^1.0.0", "workspace:*"),
            MANIFEST.replace("^1.0.0", "npm:other@1.0.0"),
            MANIFEST.replace("^1.0.0", "github:owner/left"),
            MANIFEST.replace(r#""name": "app","#, r#""name": "app", "overrides": {},"#),
            MANIFEST.replace('}', ""),
        ];
        for head in cases {
            assert_eq!(manifest_bumps(MANIFEST, &head), None, "{head}");
        }
    }

    const LOCK: &str = r#"{
      "lockfileVersion": 1,
      "workspaces": {
        "": {
          "name": "app",
          "dependencies": { "left": "^1.0.0", },
          "devDependencies": { "tool": "1.0.0", },
        },
      },
      "packages": {
        "left": ["left@1.0.0", "", { "dependencies": { "inner": "^1.0.0" } }, "sha512-left1"],
        "inner": ["inner@1.0.0", "", {}, "sha512-inner1"],
        "tool": ["tool@1.0.0", "", { "bin": { "tool-cli": "bin/cli.js" } }, "sha512-tool1"],
        "left/@s/deep": ["@s/deep@3.0.0", "", {}, "sha512-deep3"],
      }
    }
    "#;

    fn lock(text: &str) -> BunLock {
        BunLock::parse(text).expect("parses")
    }

    #[test]
    fn a_lock_whose_resolved_versions_moved_names_those_packages() {
        let head = LOCK
            .replace("inner@1.0.0", "inner@1.0.2")
            .replace("sha512-inner1", "sha512-inner2")
            .replace("@s/deep@3.0.0", "@s/deep@3.0.1");
        assert_eq!(
            lock_bumps(&lock(LOCK), &lock(&head)),
            set(&["@s/deep", "inner"])
        );
    }

    #[test]
    fn a_lock_whose_workspace_range_and_entry_moved_names_the_package() {
        let head = LOCK
            .replace(r#""tool": "1.0.0""#, r#""tool": "1.0.1""#)
            .replace("tool@1.0.0", "tool@1.0.1")
            .replace("sha512-tool1", "sha512-tool2");
        assert_eq!(lock_bumps(&lock(LOCK), &lock(&head)), set(&["tool"]));
    }

    #[test]
    fn a_lock_change_beyond_versions_is_not_a_bump() {
        let cases = [
            LOCK.replace(
                r#""inner": ["inner@1.0.0", "", {}, "sha512-inner1"],"#,
                r#""inner": ["inner@1.0.0", "", {}, "sha512-inner1"], "new": ["new@1.0.0", "", {}, "sha512-n"],"#,
            ),
            LOCK.replace(r#""inner": ["inner@1.0.0", "", {}, "sha512-inner1"],"#, ""),
            LOCK.replace(r#""lockfileVersion": 1,"#, r#""lockfileVersion": 1, "overrides": { "inner": "1.0.0" },"#),
            LOCK.replace(r#""bin": { "tool-cli""#, r#""bin": { "tool-x""#),
            LOCK.replace("left@1.0.0", "other@1.0.0"),
            LOCK.replace("left@1.0.0", "left@github:owner/left"),
            LOCK.replace(r#""", { "dependencies""#, r#""https://r.example/", { "dependencies""#),
            LOCK.replace(r#""name": "app","#, r#""name": "app2","#),
        ];
        for head in cases {
            assert_eq!(lock_bumps(&lock(LOCK), &lock(&head)), None, "{head}");
        }
    }

    #[test]
    fn a_lock_that_does_not_parse_is_refused() {
        assert!(BunLock::parse("{ \"lockfileVersion\": 1, ").is_none());
        assert!(BunLock::parse(r#"{ "lockfileVersion": 2, "packages": {} }"#).is_none());
        assert!(BunLock::parse(r#"{ "packages": {} }"#).is_none());
    }

    #[test]
    fn a_trailing_comma_inside_a_string_is_kept() {
        assert_eq!(
            without_trailing_commas(r#"{"a": ",}", "b": [1,],}"#),
            r#"{"a": ",}", "b": [1]}"#
        );
    }

    #[test]
    fn a_key_installs_under_its_last_name() {
        assert_eq!(installed("left"), "left");
        assert_eq!(installed("@s/kit"), "@s/kit");
        assert_eq!(installed("left/inner"), "inner");
        assert_eq!(installed("left/@s/deep"), "@s/deep");
        assert_eq!(installed("@s/kit/inner"), "inner");
    }

    #[test]
    fn a_moved_package_carries_every_package_that_depends_on_it() {
        let base = lock(LOCK);
        let seeds = BTreeMap::from([("inner".to_string(), "bun.lock".to_string())]);
        let all = dependents(&[&base], seeds);
        assert_eq!(all.keys().collect::<Vec<_>>(), ["inner", "left"]);
        assert_eq!(all["left"], "bun.lock");
    }

    #[test]
    fn bins_come_from_the_entry() {
        assert_eq!(
            lock(LOCK).bins("tool"),
            BTreeSet::from(["tool-cli".to_string()])
        );
        assert!(lock(LOCK).bins("left").is_empty());
    }
}
