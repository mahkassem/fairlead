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

fn parse(date: &str) -> Option<(i64, i64, i64)> {
    let mut parts = date.get(..10)?.split('-').map(|p| p.parse::<i64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

impl Window {
    pub fn ending(until: &str, window_days: u32) -> Option<Window> {
        let (y, m, d) = parse(until)?;
        let (fy, fm, fd) = civil(days(y, m, d) - i64::from(window_days));
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
            Window::ending("2026-03-01T10:00:00Z", 1).unwrap().from,
            "2026-02-28"
        );
        assert_eq!(Window::ending("2024-03-01", 1).unwrap().from, "2024-02-29");
        assert_eq!(Window::ending("2026-09-26", 90).unwrap().from, "2026-06-28");
        let w = Window::ending("2026-09-26", 90).unwrap();
        assert!(w.contains("2026-06-28T00:00:00Z") && w.contains("2026-09-26T23:59:59Z"));
        assert!(!w.contains("2026-06-27T23:59:59Z"));
    }
}
