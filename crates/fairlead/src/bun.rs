//! The Bun version a repository pins, for what init writes on it: Bun 1.4.4
//! added `bun test --parallel` and `bun fmt --check`. A version that can't be
//! read means "don't assume", so init keeps what works on every Bun.

use std::path::Path;

/// The first Bun with `bun test --parallel` and `bun fmt --check`.
pub const PARALLEL: Version = Version {
    major: 1,
    minor: 4,
    patch: 4,
    release: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// A prerelease (`1.4.4-canary.1`) sorts before its release, so it
    /// isn't taken to have what the release has. Field order makes it so.
    pub release: bool,
}

impl Version {
    pub fn at_least(self, other: Version) -> bool {
        self >= other
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.release {
            f.write_str("-pre")?;
        }
        Ok(())
    }
}

/// `1.4.4`, `bun@1.4.4`, `bun@1.4.4+sha512.…`, `v1.4.4` or `bun-v1.4.4`,
/// surrounding whitespace allowed; anything else, a range included, is None.
pub fn parse(text: &str) -> Option<Version> {
    let text = text.trim();
    let text = ["bun@", "bun-v", "v"]
        .iter()
        .find_map(|p| text.strip_prefix(p))
        .unwrap_or(text);
    let core = text.split('+').next()?;
    let (core, pre) = match core.split_once('-') {
        Some((c, p)) if !p.is_empty() => (c, true),
        Some(_) => return None,
        None => (core, false),
    };
    let mut parts = core.split('.').map(|p| {
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<u64>().ok())
            .flatten()
    });
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() {
        return None;
    }
    Some(Version {
        major,
        minor,
        patch,
        release: !pre,
    })
}

/// The version `package.json`'s `packageManager` names when it is Bun, else
/// `.bun-version`'s, with where it came from.
pub fn detect(root: &Path) -> Option<(Version, &'static str)> {
    let manager = std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|m| m["packageManager"].as_str().map(str::to_string))
        .filter(|m| m.trim_start().starts_with("bun@"))
        .and_then(|m| parse(&m));
    if let Some(v) = manager {
        return Some((v, "packageManager"));
    }
    let pinned = std::fs::read_to_string(root.join(".bun-version")).ok()?;
    parse(&pinned).map(|v| (v, ".bun-version"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
            release: true,
        }
    }

    #[test]
    fn package_manager_and_pin_file_forms_parse() {
        assert_eq!(parse("bun@1.4.4"), Some(v(1, 4, 4)));
        assert_eq!(parse("bun@1.4.4+sha512.abc"), Some(v(1, 4, 4)));
        assert_eq!(parse("1.4.4\n"), Some(v(1, 4, 4)));
        assert_eq!(parse("  bun-v1.5.0 "), Some(v(1, 5, 0)));
        assert_eq!(parse("v2.0.10"), Some(v(2, 0, 10)));
    }

    #[test]
    fn garbage_ranges_and_partial_versions_are_not_assumed() {
        for text in [
            "",
            "latest",
            "canary",
            "^1.4.4",
            ">=1.4.4",
            "1.4",
            "1.4.x",
            "1.4.4.1",
            "bun@",
            "pnpm@9.0.0",
            "1.-4.4",
            "1.4.4-",
            "1..4",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn only_1_4_4_and_later_releases_qualify() {
        assert!(parse("bun@1.4.4").unwrap().at_least(PARALLEL));
        assert!(parse("1.4.10").unwrap().at_least(PARALLEL));
        assert!(parse("1.5.0").unwrap().at_least(PARALLEL));
        assert!(parse("2.0.0").unwrap().at_least(PARALLEL));
        assert!(!parse("1.3.9").unwrap().at_least(PARALLEL));
        assert!(!parse("1.4.3").unwrap().at_least(PARALLEL));
        assert!(!parse("0.9.9").unwrap().at_least(PARALLEL));
        assert!(!parse("1.4.4-canary.3").unwrap().at_least(PARALLEL));
        assert!(parse("1.4.5-canary.1").unwrap().at_least(PARALLEL));
    }

    #[test]
    fn package_manager_wins_over_the_pin_file_and_another_manager_is_skipped() {
        let dir = std::env::temp_dir().join(format!("fairlead-bun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(detect(&dir), None);
        std::fs::write(dir.join(".bun-version"), "1.3.9\n").unwrap();
        assert_eq!(detect(&dir), Some((v(1, 3, 9), ".bun-version")));
        std::fs::write(
            dir.join("package.json"),
            r#"{"packageManager": "pnpm@9.1.0"}"#,
        )
        .unwrap();
        assert_eq!(detect(&dir), Some((v(1, 3, 9), ".bun-version")));
        std::fs::write(
            dir.join("package.json"),
            r#"{"packageManager": "bun@1.4.4+sha512.abc"}"#,
        )
        .unwrap();
        assert_eq!(detect(&dir), Some((v(1, 4, 4), "packageManager")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
