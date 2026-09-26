//! The graph-build budget from issue #17, on a generated 2,000-file
//! workspace. Ignored by default because it only means something in a
//! release build: CI runs it with `--release -- --ignored`.

use std::fmt::Write as _;
use std::fs;
use std::time::Instant;

use fairlead_core::config::{Config, EdgeRule};
use fairlead_lang::build;

const PACKAGES: usize = 40;
const FILES_PER_PACKAGE: usize = 50;
const BUDGET_SECONDS: f64 = 1.5;
const WARM_BUDGET_SECONDS: f64 = 0.5;
const FILLER_FUNCTIONS: usize = 12;
const RULES_BUDGET_SECONDS: f64 = 0.5;

#[test]
#[ignore]
fn a_two_thousand_file_workspace_builds_within_the_budget() {
    let dir = std::env::temp_dir().join(format!("fairlead-perf-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(
        dir.join("package.json"),
        r#"{ "workspaces": ["packages/*"] }"#,
    )
    .unwrap();
    for p in 0..PACKAGES {
        let pkg = dir.join(format!("packages/p{p}"));
        fs::create_dir_all(pkg.join("src")).unwrap();
        fs::write(
            pkg.join("package.json"),
            format!(r#"{{ "name": "@gen/p{p}", "exports": {{ ".": "./src/f0.ts" }} }}"#),
        )
        .unwrap();
        for f in 0..FILES_PER_PACKAGE {
            let mut src = String::new();
            if f + 1 < FILES_PER_PACKAGE {
                writeln!(src, "import {{ v{} }} from './f{}.js';", f + 1, f + 1).unwrap();
            }
            if p > 0 && f == 0 {
                writeln!(src, "import {{ v0 as dep }} from '@gen/p{}';", p - 1).unwrap();
            }
            for n in 0..FILLER_FUNCTIONS {
                writeln!(src, "export function fn{n}(a: number, b: string): string {{\n  return `${{a}}-${{b}}-{n}`;\n}}").unwrap();
            }
            writeln!(src, "export const v{f} = {f};").unwrap();
            fs::write(pkg.join(format!("src/f{f}.ts")), src).unwrap();
        }
    }
    let started = Instant::now();
    let scan = build(&dir, &Config::default()).unwrap();
    let seconds = started.elapsed().as_secs_f64();
    let stats = scan.graph.stats();
    println!(
        "{} files, {} edges, built in {seconds:.2} s",
        stats.files, stats.edges
    );
    assert_eq!(scan.tree.sources().count(), PACKAGES * FILES_PER_PACKAGE);
    assert_eq!(
        stats.edges,
        PACKAGES * (FILES_PER_PACKAGE - 1) + (PACKAGES - 1)
    );
    assert!(
        seconds <= BUDGET_SECONDS,
        "built in {seconds:.2} s, over the {BUDGET_SECONDS} s budget"
    );

    let started = Instant::now();
    let warm = build(&dir, &Config::default()).unwrap();
    let warm_seconds = started.elapsed().as_secs_f64();
    println!("rebuilt from the parse cache in {warm_seconds:.2} s");
    assert_eq!(warm.cache.hits, PACKAGES * FILES_PER_PACKAGE);
    assert_eq!(warm.graph.stats(), stats);
    assert!(
        warm_seconds <= WARM_BUDGET_SECONDS,
        "rebuilt in {warm_seconds:.2} s, over the {WARM_BUDGET_SECONDS} s budget"
    );

    // One rule per package, each linking its entry to every file beside it.
    let mut rules = Config::default();
    rules.graph.barrier = vec!["packages/p0/**".to_string()].into();
    rules.graph.edges = vec![EdgeRule {
        from: "packages/{p}/src/f0.ts".into(),
        to: vec!["packages/{p}/src/**".into()],
    }]
    .into();
    let started = Instant::now();
    let ruled = build(&dir, &rules).unwrap();
    let ruled_seconds = started.elapsed().as_secs_f64();
    println!("rebuilt with rules in {ruled_seconds:.2} s");
    assert_eq!(ruled.rules.edges, PACKAGES * (FILES_PER_PACKAGE - 1));
    assert_eq!(ruled.graph.barrier.len(), FILES_PER_PACKAGE + 1);
    assert!(
        ruled_seconds <= RULES_BUDGET_SECONDS,
        "rebuilt with rules in {ruled_seconds:.2} s, over the {RULES_BUDGET_SECONDS} s budget"
    );
}
