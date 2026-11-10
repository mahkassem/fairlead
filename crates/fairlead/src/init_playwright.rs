//! Playwright specs in `fairlead init`. A spec is named like a unit test
//! (`*.spec.ts`), so the unit runner's patterns match it, and a unit runner
//! can't run it. Init finds the specs by what they import and gives them a
//! runner of their own, which the unit runner then leaves them to.

use std::collections::BTreeSet;
use std::path::Path;

use fairlead_core::pattern::Pattern;

const MODULE: &str = "@playwright/test";
const NAMES: &str = "*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}";

/// The files `matches` finds that import Playwright's test module.
pub fn specs(root: &Path, files: &[String], matches: &[String]) -> Vec<String> {
    let patterns: Vec<Pattern> = matches
        .iter()
        .filter_map(|g| Pattern::new(g).ok())
        .collect();
    files
        .iter()
        .filter(|f| !f.contains("node_modules/"))
        .filter(|f| patterns.iter().any(|p| p.is_match(f)))
        .filter(|f| {
            std::fs::read_to_string(root.join(f)).is_ok_and(|text| {
                text.contains(&format!("\"{MODULE}\"")) || text.contains(&format!("'{MODULE}'"))
            })
        })
        .cloned()
        .collect()
}

/// Patterns for the specs: a folder's test files when every one of them is
/// a spec, such as `e2e/**/*.spec.ts`, else each spec by its path, so a
/// unit test beside a spec keeps its runner.
pub fn globs(files: &[String], matches: &[String], specs: &[String]) -> Vec<String> {
    let patterns: Vec<Pattern> = matches
        .iter()
        .filter_map(|g| Pattern::new(g).ok())
        .collect();
    let is_spec: BTreeSet<&str> = specs.iter().map(String::as_str).collect();
    let mut out: BTreeSet<String> = BTreeSet::new();
    for spec in specs {
        let Some((top, _)) = spec.split_once('/') else {
            out.insert(spec.clone());
            continue;
        };
        let under = format!("{top}/");
        let only_specs = files
            .iter()
            .filter(|f| f.starts_with(&under) && patterns.iter().any(|p| p.is_match(f)))
            .all(|f| is_spec.contains(f.as_str()));
        out.insert(match only_specs {
            true => format!("{top}/**/{NAMES}"),
            false => spec.clone(),
        });
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_folder_of_only_specs_is_one_pattern_and_a_mixed_one_names_each_spec() {
        let matches = list(&["**/*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}"]);
        let files = list(&[
            "e2e/cart.spec.ts",
            "e2e/flows/pay.spec.ts",
            "e2e/helpers.ts",
            "src/a.test.ts",
            "src/page.spec.ts",
            "root.spec.ts",
        ]);
        let specs = list(&[
            "e2e/cart.spec.ts",
            "e2e/flows/pay.spec.ts",
            "src/page.spec.ts",
            "root.spec.ts",
        ]);
        assert_eq!(
            globs(&files, &matches, &specs),
            list(&[
                "e2e/**/*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}",
                "root.spec.ts",
                "src/page.spec.ts",
            ])
        );
    }
}
