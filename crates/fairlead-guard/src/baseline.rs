//! The ratchet: how many findings of each ratcheted rule each file may keep.
//! A count can fall freely and never rise; `--write-baseline` lowers it.

use std::collections::BTreeMap;
use std::path::Path;

use crate::finding::Finding;

/// File, then rule, then count.
pub type Counts = BTreeMap<String, BTreeMap<String, u64>>;

pub fn tally<'a>(findings: impl IntoIterator<Item = &'a Finding>) -> Counts {
    let mut counts = Counts::new();
    for f in findings {
        *counts
            .entry(f.file.clone())
            .or_default()
            .entry(f.rule.to_string())
            .or_default() += 1;
    }
    counts
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub file: String,
    pub rule: String,
    pub was: u64,
    pub now: u64,
}

/// Every file and rule whose count differs, in order.
pub fn changes(now: &Counts, was: &Counts) -> Vec<Change> {
    let empty = BTreeMap::new();
    let files: std::collections::BTreeSet<&String> = now.keys().chain(was.keys()).collect();
    let mut out = Vec::new();
    for file in files {
        let (n, w) = (
            now.get(file).unwrap_or(&empty),
            was.get(file).unwrap_or(&empty),
        );
        let rules: std::collections::BTreeSet<&String> = n.keys().chain(w.keys()).collect();
        for rule in rules {
            let (now, was) = (
                n.get(rule).copied().unwrap_or(0),
                w.get(rule).copied().unwrap_or(0),
            );
            if now != was {
                out.push(Change {
                    file: file.clone(),
                    rule: rule.clone(),
                    was,
                    now,
                });
            }
        }
    }
    out
}

/// A missing file is an empty baseline.
pub fn read(path: &Path) -> Result<Counts, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Counts::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub fn write(path: &Path, counts: &Counts) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(counts).expect("counts serialize");
    std::fs::write(path, format!("{text}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(file: &str, rule: &'static str) -> Finding {
        Finding {
            file: file.into(),
            line: 1,
            rule,
            message: String::new(),
            anchor: String::new(),
            measure: None,
        }
    }

    fn counts(entries: &[(&str, &str, u64)]) -> Counts {
        let mut c = Counts::new();
        for (file, rule, n) in entries {
            c.entry(file.to_string())
                .or_default()
                .insert(rule.to_string(), *n);
        }
        c
    }

    #[test]
    fn tally_counts_per_file_and_rule() {
        let found = [finding("a", "x"), finding("a", "x"), finding("b", "x")];
        assert_eq!(tally(&found), counts(&[("a", "x", 2), ("b", "x", 1)]));
    }

    #[test]
    fn changes_name_every_pair_that_rose_or_fell_and_none_that_held() {
        let was = counts(&[("a", "x", 2), ("b", "x", 1), ("c", "x", 1)]);
        let now = counts(&[("a", "x", 3), ("c", "x", 1), ("d", "x", 1)]);
        let found: Vec<(String, u64, u64)> = changes(&now, &was)
            .into_iter()
            .map(|c| (c.file, c.was, c.now))
            .collect();
        assert_eq!(
            found,
            vec![("a".into(), 2, 3), ("b".into(), 1, 0), ("d".into(), 0, 1)]
        );
    }

    #[test]
    fn a_written_baseline_reads_back_and_a_missing_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("fairlead-baseline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("baseline.json");
        let _ = std::fs::remove_file(&path);
        assert!(read(&path).unwrap().is_empty());
        let c = counts(&[("src/a.ts", "file-length", 1)]);
        write(&path, &c).unwrap();
        assert_eq!(read(&path).unwrap(), c);
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("}\n"));
    }
}
