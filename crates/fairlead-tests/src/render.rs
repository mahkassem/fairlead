//! A plan as text, for a person reading a terminal.

use std::fmt::Write as _;

use fairlead_core::plan::{Plan, Reason};

pub fn reason(reason: &Reason) -> String {
    match reason {
        Reason::RunAll { path } => format!("run_all: {path}"),
        Reason::Changed => "changed".into(),
        Reason::Import { chain } => {
            format!("{} hops from {}", chain.len().saturating_sub(1), chain[0])
        }
        Reason::Owner {
            rule,
            covers,
            changed,
        } => {
            format!("owner rule {}: covers {covers} ({changed})", rule + 1)
        }
        Reason::Canary => "canary".into(),
        Reason::Unreached { path, policy } => format!("unreached {path} ({policy})"),
        Reason::Paths { matched } => format!("paths matched {matched} changed files"),
        Reason::Modules { modules } => format!("modules: {}", modules.join(", ")),
        Reason::Always => "always".into(),
    }
}

pub fn text(plan: &Plan) -> String {
    let mut out = String::new();
    let scope = if plan.all {
        "everything"
    } else {
        "a selection"
    };
    let _ = writeln!(
        out,
        "plan {}: {} changed, {} tests, {} checks ({scope})",
        plan.plan_id,
        plan.changed.len(),
        plan.tests.len(),
        plan.checks.len()
    );
    if !plan.tests.is_empty() {
        let _ = writeln!(out, "\ntests:");
        let width = plan.tests.iter().map(|t| t.path.len()).max().unwrap_or(0);
        for test in &plan.tests {
            let _ = writeln!(out, "  {:width$}  {}", test.path, reason(&test.reason));
        }
    }
    if !plan.checks.is_empty() {
        let _ = writeln!(out, "\nchecks:");
        for check in &plan.checks {
            let _ = writeln!(out, "  {}  {}", check.id, reason(&check.reason));
        }
    }
    if !plan.unreached.is_empty() {
        let _ = writeln!(out, "\nunreached (no test depends on these):");
        for file in &plan.unreached {
            let _ = writeln!(out, "  {}  selected: {}", file.path, file.selected);
        }
    }
    if !plan.ignored.is_empty() {
        let _ = writeln!(out, "\nignored: {}", plan.ignored.join(", "));
    }
    if !plan.invocations.is_empty() {
        let _ = writeln!(out, "\nrun:");
        for inv in &plan.invocations {
            let _ = writeln!(out, "  ({}) {}", inv.cwd, inv.argv.join(" "));
        }
    }
    let other: Vec<_> = plan
        .warnings
        .iter()
        .filter(|w| w.code != "unreached")
        .collect();
    if !other.is_empty() {
        let _ = writeln!(out, "\nwarnings:");
        for w in other {
            let path = w
                .path
                .as_deref()
                .map(|p| format!("{p}: "))
                .unwrap_or_default();
            let _ = writeln!(out, "  [{}] {path}{}", w.code, w.message);
        }
    }
    out
}
