//! The done receipt on a pull request: the block `fairlead receipt --pr`
//! prints for the body, read back by `ci report` for its comment and by
//! `pr check` to tie the body's claims to a passing receipt for the head.

use std::path::Path;

use fairlead_guard::claims::{self, BodyReceipt};

use crate::receipt_cmd::{self, Receipt};

/// The gate's state as one word, for the block's marker.
pub fn gate_word(state: &str) -> &'static str {
    match state {
        "passed" => "passed",
        "failed" => "failed",
        "not run" => "not-run",
        _ => "waits",
    }
}

/// The receipt as a block for a pull request body.
pub fn block(r: &Receipt) -> String {
    let summary = format!(
        "gate {} for tree {}",
        match gate_word(&r.gate.state) {
            "not-run" => "not run",
            word => word,
        },
        r.tree.chars().take(12).collect::<String>()
    );
    claims::block(
        &r.tree,
        gate_word(&r.gate.state),
        &summary,
        &receipt_cmd::text(r, false),
    )
}

/// The receipts a file holds: a receipt as JSON, the blocks of a body, or
/// a plain text receipt whose tree isn't known.
pub fn read_file(path: &Path) -> Result<Vec<BodyReceipt>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    if let Ok(r) = serde_json::from_str::<Receipt>(&text) {
        return Ok(vec![BodyReceipt {
            tree: r.tree.clone(),
            gate: gate_word(&r.gate.state).into(),
            text: receipt_cmd::text(&r, false).trim_end().to_string(),
        }]);
    }
    let found = claims::receipts(&text);
    if !found.is_empty() {
        return Ok(found);
    }
    Ok(vec![BodyReceipt {
        tree: String::new(),
        gate: String::new(),
        text: text.trim_end().to_string(),
    }])
}

/// The pull request's body from GitHub's event payload, when there is one.
pub fn event_body() -> Option<String> {
    let path = std::env::var_os("GITHUB_EVENT_PATH")?;
    let payload: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    payload["pull_request"]["body"].as_str().map(String::from)
}

/// The tree of the pull request's head commit in CI, else of HEAD.
pub fn head_tree(root: &Path) -> Option<String> {
    let head = std::env::var_os("GITHUB_EVENT_PATH")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["pull_request"]["head"]["sha"].as_str().map(String::from))
        .unwrap_or_else(|| "HEAD".into());
    tree_of(root, &head)
}

/// A commit's tree id.
pub fn tree_of(root: &Path, rev: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
        .arg(format!("{rev}^{{tree}}"))
        .current_dir(root)
        .output()
        .ok()?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !id.is_empty()).then_some(id)
}

/// What a report's receipt section is called: whether it is for the
/// head's tree, so an older receipt isn't read as this head's.
pub fn label(r: &BodyReceipt, head: Option<&str>) -> String {
    match (r.tree.as_str(), head) {
        ("", _) | (_, None) => "Receipt".into(),
        (tree, Some(h)) if tree == h => format!("Receipt for the head's tree: gate {}", r.gate),
        (tree, Some(_)) => format!(
            "Receipt for an earlier tree ({}), not the head's",
            tree.chars().take(12).collect::<String>()
        ),
    }
}
