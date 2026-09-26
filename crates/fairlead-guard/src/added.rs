//! Which findings an edit adds. An edit is judged by what it changes, so a
//! finding already in the file doesn't block an unrelated fix; the check
//! stage still reports it against the baseline.

use std::collections::HashMap;

use crate::finding::Finding;

/// The findings in `after` that `before` doesn't account for. Findings
/// compare as multisets of (rule, message, anchor), so one that only moved
/// lines isn't new and two identical ones count twice. A measured finding
/// is new when nothing of its rule and anchor was over the limit before,
/// or when it grew.
pub fn added(before: &[Finding], after: &[Finding]) -> Vec<Finding> {
    let mut plain: HashMap<(&str, &str, &str), usize> = HashMap::new();
    let mut measured: HashMap<(&str, &str), Vec<&Finding>> = HashMap::new();
    for f in before {
        match f.measure {
            None => *plain.entry((f.rule, &f.message, &f.anchor)).or_default() += 1,
            Some(_) => measured.entry((f.rule, &f.anchor)).or_default().push(f),
        }
    }
    let mut new = Vec::new();
    let mut grown: HashMap<(&str, &str), Vec<&Finding>> = HashMap::new();
    for f in after {
        match f.measure {
            None => match plain.get_mut(&(f.rule, f.message.as_str(), f.anchor.as_str())) {
                Some(n) if *n > 0 => *n -= 1,
                _ => new.push(f.clone()),
            },
            Some(_) => grown.entry((f.rule, &f.anchor)).or_default().push(f),
        }
    }
    // Same-named things pair in the order they appear, which an edit
    // elsewhere in the file doesn't change.
    let size = |f: &Finding| f.measure.map_or(0, |m| m.size);
    for (key, mut now) in grown {
        now.sort_by_key(|f| f.line);
        let mut was = measured.remove(&key).unwrap_or_default();
        was.sort_by_key(|f| f.line);
        for (i, f) in now.into_iter().enumerate() {
            if was.get(i).is_none_or(|w| size(f) > size(w)) {
                new.push(f.clone());
            }
        }
    }
    crate::finding::sort(&mut new);
    new
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Measure;

    fn plain(line: u32, message: &str, anchor: &str) -> Finding {
        Finding {
            file: "a.ts".into(),
            line,
            rule: "history",
            message: message.into(),
            anchor: anchor.into(),
            measure: None,
        }
    }

    fn sized(line: u32, anchor: &str, size: u64) -> Finding {
        Finding {
            file: "a.ts".into(),
            line,
            rule: "function-length",
            message: format!("{size} lines"),
            anchor: anchor.into(),
            measure: Some(Measure { size, limit: 10 }),
        }
    }

    #[test]
    fn a_finding_that_only_moved_lines_is_not_new() {
        let before = [plain(3, "a date", "x")];
        let after = [plain(9, "a date", "x")];
        assert!(added(&before, &after).is_empty());
    }

    #[test]
    fn a_second_identical_finding_is_new() {
        let before = [plain(3, "a date", "x")];
        let after = [plain(3, "a date", "x"), plain(7, "a date", "x")];
        assert_eq!(added(&before, &after), vec![plain(7, "a date", "x")]);
    }

    #[test]
    fn a_different_message_or_anchor_is_new() {
        let before = [plain(3, "a date", "x")];
        let after = [plain(3, "a name", "x"), plain(4, "a date", "y")];
        assert_eq!(added(&before, &after).len(), 2);
    }

    #[test]
    fn a_measured_finding_is_new_when_it_crosses_the_limit_or_grows() {
        assert_eq!(added(&[], &[sized(1, "f", 12)]).len(), 1);
        assert_eq!(added(&[sized(1, "f", 12)], &[sized(5, "f", 13)]).len(), 1);
    }

    #[test]
    fn a_measured_finding_that_stays_or_shrinks_is_not_new() {
        assert!(added(&[sized(1, "f", 12)], &[sized(5, "f", 12)]).is_empty());
        assert!(added(&[sized(1, "f", 12)], &[sized(1, "f", 11)]).is_empty());
    }

    #[test]
    fn same_named_measured_findings_pair_in_the_order_they_appear() {
        let before = [sized(1, "f", 20), sized(40, "f", 12)];
        let shrank_and_grew = [sized(3, "f", 15), sized(45, "f", 21)];
        assert_eq!(added(&before, &shrank_and_grew), vec![sized(45, "f", 21)]);
        let both_shrank = [sized(1, "f", 19), sized(40, "f", 11)];
        assert!(added(&before, &both_shrank).is_empty());
    }
}
