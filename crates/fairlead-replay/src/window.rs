//! The replay window: `window_days` back from the newest recorded run, or
//! from `--until`, so the same dataset always gives the same report.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// Inclusive bounds, as `YYYY-MM-DD`.
    pub from: String,
    pub until: String,
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// `YYYY-MM-DD` shifted by `by` days.
pub fn add_days(date: &str, by: i64) -> Option<String> {
    let (y, m, d) = parse(date)?;
    let (y, m, d) = civil(days(y, m, d) + by);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

fn parse(date: &str) -> Option<(i64, i64, i64)> {
    let mut parts = date.get(..10)?.split('-').map(|p| p.parse::<i64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Seconds since the epoch for an RFC 3339 UTC timestamp such as
/// `2026-11-07T10:00:00Z`; fractions and an offset other than `Z` are ignored.
pub fn epoch_seconds(timestamp: &str) -> Option<i64> {
    let (y, m, d) = parse(timestamp)?;
    let time = timestamp.get(11..19)?;
    let mut hms = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi, s) = (hms.next()??, hms.next()??, hms.next()??);
    Some(days(y, m, d) * 86_400 + h * 3_600 + mi * 60 + s)
}

/// Seconds between two timestamps, when both parse and the second isn't earlier.
pub fn seconds_between(from: &str, to: &str) -> Option<u64> {
    let span = epoch_seconds(to)? - epoch_seconds(from)?;
    u64::try_from(span).ok()
}

impl Window {
    pub fn ending(until: &str, window_days: u32) -> Option<Window> {
        let (y, m, d) = parse(until)?;
        let span = i64::from(window_days.max(1)) - 1;
        let (fy, fm, fd) = civil(days(y, m, d) - span);
        Some(Window {
            from: format!("{fy:04}-{fm:02}-{fd:02}"),
            until: until[..10].to_string(),
        })
    }

    pub fn contains(&self, timestamp: &str) -> bool {
        let date = timestamp.get(..10).unwrap_or("");
        date >= self.from.as_str() && date <= self.until.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_counts_back_across_months_and_leap_days() {
        assert_eq!(
            Window::ending("2026-03-01T10:00:00Z", 2).unwrap().from,
            "2026-02-28"
        );
        assert_eq!(Window::ending("2024-03-01", 2).unwrap().from, "2024-02-29");
        assert_eq!(Window::ending("2026-09-26", 1).unwrap().from, "2026-09-26");
        let w = Window::ending("2026-09-26", 90).unwrap();
        assert_eq!(w.from, "2026-06-29", "90 days, both ends included");
        assert!(w.contains("2026-06-29T00:00:00Z") && w.contains("2026-09-26T23:59:59Z"));
        assert!(!w.contains("2026-06-28T23:59:59Z"));
    }

    #[test]
    fn a_timestamp_becomes_seconds_and_a_span_between_two() {
        assert_eq!(epoch_seconds("1970-01-02T00:00:01Z"), Some(86_401));
        assert_eq!(
            seconds_between("2026-11-07T23:59:30Z", "2026-11-08T00:01:00Z"),
            Some(90)
        );
        assert_eq!(seconds_between("2026-11-08T00:00:00Z", "2026-11-07T00:00:00Z"), None);
        assert_eq!(epoch_seconds("2026-11-08"), None);
    }
}
