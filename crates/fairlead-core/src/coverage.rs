//! A coverage map: which source files each test ran, recorded by a
//! coverage run and imported into this one shape. The plan adds its edges
//! to the static graph, so dependencies only a run shows (a container, a
//! dynamic import, framework wiring) reach their tests. A public contract,
//! versioned like the plan.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CoverageMap {
    /// Always 1 for this shape.
    pub version: u32,
    /// The commit the coverage run tested.
    pub commit: String,
    /// When the map was made, `YYYY-MM-DD`.
    pub created: String,
    /// What it was imported from, such as `phpunit-xml` or `coverage-py`.
    pub source: String,
    /// Each test file, repo-relative, and the repo-relative source files it ran.
    pub tests: BTreeMap<String, Vec<String>>,
}

pub fn json_schema() -> serde_json::Value {
    crate::plan::sorted(
        serde_json::to_value(schemars::schema_for!(CoverageMap)).expect("schema serializes"),
    )
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date, by Howard Hinnant's
/// `days_from_civil`.
pub fn days(date: &str) -> Option<i64> {
    let mut parts = date.get(..10)?.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Today in UTC, `YYYY-MM-DD`.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_count_days_both_ways() {
        assert_eq!(days("1970-01-01"), Some(0));
        assert_eq!(
            days("2026-09-28")
                .zip(days("2026-10-05"))
                .map(|(a, b)| b - a),
            Some(7)
        );
        assert_eq!(
            days("2024-03-01")
                .zip(days("2024-02-28"))
                .map(|(a, b)| a - b),
            Some(2)
        );
        assert_eq!(days("soon"), None);
        let t = today();
        assert!(
            days(&t).is_some_and(|d| d > days("2026-01-01").unwrap()),
            "{t}"
        );
    }
}
