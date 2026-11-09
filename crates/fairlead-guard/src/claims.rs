//! A pull request body's claims that the change was verified, and the
//! receipts `fairlead receipt --pr` puts in a body. A claim needs a passing
//! receipt for the head's tree; the words are a short, generic list.

use std::sync::OnceLock;

use regex::Regex;

/// The start of a receipt block, followed by `tree=… gate=… -->`.
pub const OPEN: &str = "<!-- fairlead-receipt";
pub const CLOSE: &str = "<!-- /fairlead-receipt -->";

/// A receipt found in a body: the tree it is for and the gate's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyReceipt {
    pub tree: String,
    /// `passed`, `failed`, `waits` or `not-run`.
    pub gate: String,
    /// The receipt's text, as `fairlead receipt` printed it.
    pub text: String,
}

impl BodyReceipt {
    pub fn passed_for(&self, tree: &str) -> bool {
        self.tree == tree && self.gate == "passed"
    }
}

fn words() -> &'static Regex {
    static WORDS: OnceLock<Regex> = OnceLock::new();
    WORDS.get_or_init(|| {
        Regex::new(
            r"(?i)\b(verified|tested|(?:all )?(?:tests|checks) (?:pass|passed|passing|are green)|all green|works as expected)\b",
        )
        .expect("valid")
    })
}

/// Whether the words just before a match deny it, as in "not tested".
fn denied(before: &str) -> bool {
    before
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .rev()
        .take(3)
        .any(|w| matches!(w, "not" | "never" | "no" | "without") || w.ends_with("n't"))
}

/// The claims a body makes, each once, in the order they appear. Code
/// blocks, HTML comments, receipt blocks and unticked task items are not
/// claims.
pub fn claims(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let (mut fenced, mut comment, mut receipt) = (false, false, false);
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with(OPEN) || t.starts_with(CLOSE) {
            receipt = t.starts_with(OPEN);
            continue;
        }
        if comment {
            comment = !t.contains("-->");
            continue;
        }
        if t.starts_with("<!--") {
            comment = !t.contains("-->");
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if receipt || fenced || t.starts_with("- [ ]") || t.starts_with("* [ ]") {
            continue;
        }
        for m in words().find_iter(line) {
            let word = m.as_str().to_lowercase();
            if !denied(&line[..m.start()]) && !out.contains(&word) {
                out.push(word);
            }
        }
    }
    out
}

/// The receipt blocks in a body.
pub fn receipts(body: &str) -> Vec<BodyReceipt> {
    let mut out = Vec::new();
    let mut open: Option<(String, String, Vec<&str>)> = None;
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(OPEN) {
            let field = |key: &str| {
                rest.split_whitespace()
                    .find_map(|w| w.strip_prefix(key))
                    .map(|v| v.trim_end_matches("-->").to_string())
                    .unwrap_or_default()
            };
            open = Some((field("tree="), field("gate="), Vec::new()));
            continue;
        }
        if t.starts_with(CLOSE) {
            if let Some((tree, gate, lines)) = open.take() {
                let text = lines
                    .into_iter()
                    .filter(|l| {
                        let l = l.trim();
                        !l.starts_with("```") && !l.starts_with("<details") && l != "</details>"
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push(BodyReceipt {
                    tree,
                    gate,
                    text: text.trim().to_string(),
                });
            }
            continue;
        }
        if let Some((_, _, lines)) = open.as_mut() {
            lines.push(line);
        }
    }
    out
}

/// A receipt as a block for a pull request body.
pub fn block(tree: &str, gate: &str, summary: &str, text: &str) -> String {
    format!(
        "{OPEN} tree={tree} gate={gate} -->\n<details><summary>Fairlead receipt: {summary}</summary>\n\n```text\n{}\n```\n\n</details>\n{CLOSE}\n",
        text.trim_end()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_are_found_once_and_denials_code_and_open_tasks_are_not_claims() {
        let body = "## Summary\nVerified on Linux; all tests pass.\nTested again.\n\
                    - [ ] tested on Windows\n- [x] checks passed\n\
                    Not tested on macOS, and it wasn't verified there.\n\
                    ```\ncargo test # tested\n```\n<!-- tested -->\nuntested edge.\n";
        assert_eq!(
            claims(body),
            ["verified", "all tests pass", "tested", "checks passed"]
        );
        assert!(claims("A refactor with no behaviour change.").is_empty());
        assert!(claims("This is not verified.").is_empty());
    }

    #[test]
    fn a_receipt_block_round_trips_and_its_text_is_no_claim() {
        let made = block(
            "abc123",
            "passed",
            "passed",
            "gate     passed\nverified line",
        );
        let body = format!("Fixes a bug.\n\n{made}");
        let found = receipts(&body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tree, "abc123");
        assert_eq!(found[0].gate, "passed");
        assert_eq!(found[0].text, "gate     passed\nverified line");
        assert!(found[0].passed_for("abc123") && !found[0].passed_for("other"));
        assert!(claims(&body).is_empty(), "{:?}", claims(&body));
    }
}
