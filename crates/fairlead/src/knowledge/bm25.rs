//! Okapi BM25 over two fields, kept in memory and rebuilt per run: a corpus
//! of a few thousand short entries indexes in milliseconds, so there's
//! nothing to keep fresh on disk.
//!
//! The title field counts twice: its term frequencies and its length are
//! doubled before the usual formula, the simple form of BM25F.

use std::collections::HashMap;

const K1: f64 = 1.2;
const B: f64 = 0.75;
const TITLE_WEIGHT: f64 = 2.0;

/// Words too common to tell entries apart.
const STOP: [&str; 31] = [
    "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "for", "from", "has", "have",
    "if", "in", "into", "is", "it", "its", "no", "not", "of", "on", "or", "so", "that", "the",
    "this", "to", "with",
];

/// Lowercase terms: words split at anything not a letter, digit or `_`,
/// and a camelCase or snake_case word also as its parts, so `retryBudget`
/// is found by `retry`, `budget` and `retrybudget`. One-letter terms and
/// stop words are dropped.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        let whole = word.trim_matches('_');
        if whole.is_empty() {
            continue;
        }
        keep(&mut out, whole.to_lowercase());
        let parts = parts(whole);
        if parts.len() > 1 {
            for part in parts {
                keep(&mut out, part.to_lowercase());
            }
        }
    }
    out
}

fn keep(out: &mut Vec<String>, term: String) {
    if term.chars().nth(1).is_some() && !STOP.contains(&term.as_str()) {
        out.push(term);
    }
}

/// A word's snake_case and camelCase parts; an acronym stays whole, so
/// `HTTPServer` is `HTTP` and `Server`.
fn parts(word: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for piece in word.split('_').filter(|p| !p.is_empty()) {
        let chars: Vec<(usize, char)> = piece.char_indices().collect();
        let mut start = 0;
        for i in 1..chars.len() {
            let (at, c) = chars[i];
            let prev = chars[i - 1].1;
            let next_lower = chars.get(i + 1).is_some_and(|&(_, n)| n.is_lowercase());
            let upper_after_lower = c.is_uppercase() && !prev.is_uppercase();
            let acronym_end = c.is_uppercase() && prev.is_uppercase() && next_lower;
            if upper_after_lower || acronym_end {
                out.push(&piece[start..at]);
                start = at;
            }
        }
        out.push(&piece[start..]);
    }
    out
}

/// The terms of every entry, by term: (entry, weighted frequency).
pub struct Index {
    postings: HashMap<String, Vec<(u32, f64)>>,
    lengths: Vec<f64>,
    average: f64,
}

impl Index {
    /// Indexes entries given as (title, body).
    pub fn new<'a>(entries: impl Iterator<Item = (&'a str, &'a str)>) -> Index {
        let mut postings: HashMap<String, Vec<(u32, f64)>> = HashMap::new();
        let mut lengths = Vec::new();
        for (id, (title, body)) in entries.enumerate() {
            let mut counts: HashMap<String, f64> = HashMap::new();
            let mut length = 0.0;
            for (text, weight) in [(title, TITLE_WEIGHT), (body, 1.0)] {
                for term in tokens(text) {
                    *counts.entry(term).or_default() += weight;
                    length += weight;
                }
            }
            for (term, tf) in counts {
                postings.entry(term).or_default().push((id as u32, tf));
            }
            lengths.push(length);
        }
        let average = if lengths.is_empty() {
            1.0
        } else {
            (lengths.iter().sum::<f64>() / lengths.len() as f64).max(1.0)
        };
        Index {
            postings,
            lengths,
            average,
        }
    }

    /// Every entry that holds a term of `query`, with its score; unsorted.
    pub fn search(&self, query: &str) -> Vec<(usize, f64)> {
        let mut terms = tokens(query);
        terms.sort();
        terms.dedup();
        let n = self.lengths.len() as f64;
        let mut scores: HashMap<u32, f64> = HashMap::new();
        for term in &terms {
            let Some(list) = self.postings.get(term) else {
                continue;
            };
            let df = list.len() as f64;
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            for &(id, tf) in list {
                let norm = 1.0 - B + B * self.lengths[id as usize] / self.average;
                *scores.entry(id).or_default() += idf * tf * (K1 + 1.0) / (tf + K1 * norm);
            }
        }
        scores.into_iter().map(|(id, s)| (id as usize, s)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best(index: &Index, query: &str) -> Vec<usize> {
        let mut hits = index.search(query);
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        hits.into_iter().map(|(i, _)| i).collect()
    }

    #[test]
    fn camel_case_words_are_kept_whole_and_split() {
        assert_eq!(
            tokens("retryBudget HTTPServer parseJSON2Html"),
            [
                "retrybudget",
                "retry",
                "budget",
                "httpserver",
                "http",
                "server",
                "parsejson2html",
                "parse",
                "json2",
                "html"
            ]
        );
    }

    #[test]
    fn snake_case_words_are_kept_whole_and_split_and_stop_words_drop() {
        assert_eq!(
            tokens("The max_retry_count is __private__, a b"),
            ["max_retry_count", "max", "retry", "count", "private"]
        );
    }

    #[test]
    fn a_rarer_term_outranks_a_common_one() {
        let docs = [
            ("one", "cache cache invoice"),
            ("two", "cache cache ledger"),
            ("three", "cache cache report"),
            ("four", "cache cache summary"),
        ];
        let index = Index::new(docs.iter().map(|(t, b)| (*t, *b)));
        // Every entry has "cache"; only the second has "ledger".
        assert_eq!(best(&index, "cache ledger")[0], 1);
        let scores: HashMap<usize, f64> = index.search("cache ledger").into_iter().collect();
        assert!(scores[&1] > scores[&0] * 1.5, "{scores:?}");
    }

    #[test]
    fn a_term_in_the_title_outranks_the_same_term_in_the_body() {
        let docs = [
            ("migrations", "never edit one after it ran"),
            ("deploys", "a migrations folder is read first"),
        ];
        let index = Index::new(docs.iter().map(|(t, b)| (*t, *b)));
        assert_eq!(best(&index, "migrations"), [0, 1]);
    }

    #[test]
    fn a_query_with_no_known_term_finds_nothing() {
        let index = Index::new([("alpha", "beta")].into_iter());
        assert!(index.search("gamma the").is_empty());
        assert!(Index::new(std::iter::empty()).search("alpha").is_empty());
    }
}
