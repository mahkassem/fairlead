//! Checks: selected when their `paths` match a changed file (deleted ones
//! included, since removing a file can break a typecheck), when they watch
//! a module the change affects, or always when they name neither.

use std::collections::BTreeSet;

use fairlead_core::plan::{CheckSelection, Reason};

use crate::pattern::Pattern;
use crate::planner::Context;
use crate::walk::Walk;

pub fn checks(
    cx: &Context,
    walked: &Walk,
    all_reason: Option<&Reason>,
) -> Result<Vec<CheckSelection>, String> {
    let graph = &cx.scan.graph;
    let affected: BTreeSet<&str> = walked
        .reached
        .keys()
        .filter_map(|id| cx.modules.name_of(&graph.files[*id as usize]))
        .collect();
    let mut out = Vec::new();
    for check in cx.config.checks.items() {
        let reason = if let Some(reason) = all_reason {
            Some(reason.clone())
        } else if check.paths.is_empty() && check.modules.is_empty() {
            Some(Reason::Always)
        } else {
            let paths: Vec<Pattern> = check
                .paths
                .iter()
                .map(|g| Pattern::new(g))
                .collect::<Result<_, _>>()?;
            let matched = cx
                .changed_or_scoped()
                .filter(|p| paths.iter().any(|g| g.is_match(p)))
                .count();
            let modules: Vec<String> = check
                .modules
                .iter()
                .filter(|m| affected.contains(m.as_str()))
                .cloned()
                .collect();
            if matched > 0 {
                Some(Reason::Paths { matched })
            } else if !modules.is_empty() {
                Some(Reason::Modules { modules })
            } else {
                None
            }
        };
        if let Some(reason) = reason {
            out.push(CheckSelection {
                id: check.id.clone(),
                reason,
            });
        }
    }
    Ok(out)
}
