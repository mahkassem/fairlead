//! Which findings an edit adds. An edit is judged by what it changes, so a
//! finding already in the file doesn't block an unrelated fix; the check
//! stage still reports it against the baseline.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

use crate::finding::Finding;

/// The findings in `after` that `before` doesn't account for. Findings
/// compare as multisets of (rule, message, anchor), so one that only moved
/// lines isn't new and two identical ones count twice; what's left then
/// compares without the anchor, so rewording a comment that keeps its
/// finding doesn't add one. A measured finding pairs the same way, in the
/// order they appear, and is new when it has no partner or grew.
pub fn added(before: &[Finding], after: &[Finding]) -> Vec<Finding> {
    let exact = |f: &Finding| (f.rule, f.message.clone(), f.anchor.clone());
    let loose = |f: &Finding| (f.rule, f.message.clone(), String::new());
    let (_, was, now) = matched(by_line(before, false), by_line(after, false), exact);
    let (_, _, mut new) = matched(was, now, loose);

    let size = |f: &Finding| f.measure.map_or(0, |m| m.size);
    let (pairs, was, now) = matched(by_line(before, true), by_line(after, true), |f| {
        (f.rule, f.anchor.clone())
    });
    let (more, _, unpaired) = matched(was, now, |f| (f.rule, String::new()));
    new.extend(unpaired);
    new.extend(
        pairs
            .into_iter()
            .chain(more)
            .filter(|(w, n)| size(n) > size(w))
            .map(|(_, n)| n),
    );

    let mut new: Vec<Finding> = new.into_iter().cloned().collect();
    crate::finding::sort(&mut new);
    new
}

/// The measured or unmeasured findings, in the order they appear.
fn by_line(findings: &[Finding], sized: bool) -> Vec<&Finding> {
    let mut out: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.measure.is_some() == sized)
        .collect();
    out.sort_by_key(|f| f.line);
    out
}

type Pairs<'a> = Vec<(&'a Finding, &'a Finding)>;

/// Pairs each finding in `now` with the first unpaired one in `was` of the
/// same key, and returns the pairs and what's left on each side, in order.
fn matched<'a, K: Hash + Eq>(
    was: Vec<&'a Finding>,
    now: Vec<&'a Finding>,
    key: impl Fn(&Finding) -> K,
) -> (Pairs<'a>, Vec<&'a Finding>, Vec<&'a Finding>) {
    let mut open: HashMap<K, VecDeque<usize>> = HashMap::new();
    for (i, f) in was.iter().enumerate() {
        open.entry(key(f)).or_default().push_back(i);
    }
    let mut used = vec![false; was.len()];
    let (mut pairs, mut left_now) = (Vec::new(), Vec::new());
    for f in now {
        match open.get_mut(&key(f)).and_then(VecDeque::pop_front) {
            Some(i) => {
                used[i] = true;
                pairs.push((was[i], f));
            }
            None => left_now.push(f),
        }
    }
    let left_was = was
        .into_iter()
        .zip(used)
        .filter(|(_, u)| !u)
        .map(|(f, _)| f)
        .collect();
    (pairs, left_was, left_now)
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
    fn a_different_message_is_new() {
        let before = [plain(3, "a date", "x")];
        let after = [plain(3, "a name", "x")];
        assert_eq!(added(&before, &after).len(), 1);
    }

    #[test]
    fn rewording_a_comment_that_keeps_its_finding_adds_none() {
        let before = [plain(3, "a date", "old words"), plain(9, "a date", "other")];
        let after = [plain(3, "a date", "new words"), plain(9, "a date", "other")];
        assert!(added(&before, &after).is_empty());
        let one_more = [
            plain(3, "a date", "new words"),
            plain(5, "a date", "x"),
            plain(9, "a date", "other"),
        ];
        assert_eq!(added(&before, &one_more).len(), 1);
    }

    #[test]
    fn a_measured_thing_edited_inside_pairs_by_order_and_counts_only_growth() {
        let before = [sized(1, "old text", 12), sized(30, "g", 15)];
        let shrank = [sized(1, "new text", 11), sized(30, "g", 15)];
        assert!(added(&before, &shrank).is_empty());
        let grew = [sized(1, "new text", 13), sized(30, "g", 15)];
        assert_eq!(added(&before, &grew), vec![sized(1, "new text", 13)]);
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
