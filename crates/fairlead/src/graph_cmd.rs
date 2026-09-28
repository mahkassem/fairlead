//! `fairlead graph`: numbers about the import graph, and questions about
//! one file.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::Subcommand;
use fairlead_core::config::{self, LoadOptions};
use fairlead_lang::{build, Scan};

#[derive(Subcommand)]
pub enum GraphAction {
    /// Files, edges and what didn't resolve, with the build time.
    Stats {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// The chain by which FROM depends on TO.
    Why { from: String, to: String },
    /// The files that depend on FILE directly.
    Importers { file: String },
}

/// The repository root: the nearest directory with `.git`, else `start`.
pub fn repo_root(start: &Path) -> PathBuf {
    start
        .ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(start)
        .to_path_buf()
}

fn relative_to(root: &Path, cwd: &Path, file: &str) -> String {
    let abs = cwd.join(file);
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let abs = std::fs::canonicalize(&abs).unwrap_or(abs);
    fairlead_lang::tree::relative(&root, &abs).unwrap_or_else(|| file.replace('\\', "/"))
}

pub fn run(action: GraphAction, sets: Vec<String>, cwd: &Path) -> ExitCode {
    let loaded = match config::load(cwd, &LoadOptions::from_process(sets)) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let root = if loaded.files.is_empty() {
        repo_root(cwd)
    } else {
        loaded.root.clone()
    };
    let started = Instant::now();
    let scan = match build(&root, &loaded.config) {
        Ok(scan) => scan,
        Err(e) => {
            eprintln!("could not read {}: {e}", root.display());
            return ExitCode::FAILURE;
        }
    };
    let elapsed = started.elapsed();
    match action {
        GraphAction::Stats { json } => {
            let cache_off = if loaded.config.graph.cache {
                "off (no git directory to keep it in)"
            } else {
                "off (graph.cache = false)"
            };
            stats(&scan, elapsed.as_secs_f64(), json, cache_off)
        }
        GraphAction::Why { from, to } => why(
            &scan,
            &relative_to(&root, cwd, &from),
            &relative_to(&root, cwd, &to),
        ),
        GraphAction::Importers { file } => importers(&scan, &relative_to(&root, cwd, &file)),
    }
}

fn stats(scan: &Scan, seconds: f64, json: bool, cache_off: &str) -> ExitCode {
    let s = scan.graph.stats();
    let sources = scan.tree.sources().count();
    if json {
        let by_kind: serde_json::Map<String, serde_json::Value> = s
            .edges_by_kind
            .iter()
            .map(|(k, n)| (format!("{k:?}").to_lowercase(), (*n).into()))
            .collect();
        let value = serde_json::json!({
            "files": s.files, "sources": sources, "edges": s.edges, "edges_by_kind": by_kind,
            "package_edges": s.package_edges, "packages": s.packages, "unresolved": s.unresolved,
            "unknown_dynamic": s.unknown, "tsconfig_fallbacks": s.tsconfig_fallbacks, "seconds": seconds,
            "rules": { "edges": scan.rules.edges, "unmatched": scan.rules.unmatched, "large": scan.rules.large, "barrier": scan.graph.barrier.len() },
            "cache": { "enabled": scan.cache.enabled, "hits": scan.cache.hits, "misses": scan.cache.misses },
            "providers": scan.providers.iter().map(|p| serde_json::json!({
                "id": p.id, "files": p.files, "edges": p.edges, "ignored": p.ignored, "failed": p.failed,
            })).collect::<Vec<_>>(),
            "conflicts": scan.conflicts.len(),
            "coverage": scan.coverage.as_ref().map(|c| serde_json::json!({
                "map": c.map, "commit": c.commit, "created": c.created, "source": c.source,
                "tests": c.tests, "edges": c.edges, "ignored": c.ignored, "error": c.error,
            })),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("stats print")
        );
    } else {
        println!("files: {} ({} sources)", s.files, sources);
        let kinds: Vec<String> = s
            .edges_by_kind
            .iter()
            .map(|(k, n)| format!("{} {}", format!("{k:?}").to_lowercase(), n))
            .collect();
        println!("edges: {} ({})", s.edges, kinds.join(", "));
        if scan.providers.len() > 1 {
            let each: Vec<String> = scan
                .providers
                .iter()
                .map(|p| match &p.failed {
                    Some(why) => format!("{} failed on {} files: {why}", p.id, p.files),
                    None => format!("{} {} edges from {} files", p.id, p.edges, p.files),
                })
                .collect();
            println!("providers: {}", each.join("; "));
            for p in scan.providers.iter().filter(|p| p.ignored > 0) {
                println!(
                    "  {} printed {} edges naming a file outside the tree or its claim",
                    p.id, p.ignored
                );
            }
            if let Some(c) = scan.conflicts.first() {
                println!(
                    "  {} files claimed by two providers, such as {} ({} over {})",
                    scan.conflicts.len(),
                    c.file,
                    c.chosen,
                    c.other
                );
            }
        }
        if let Some(c) = &scan.coverage {
            match &c.error {
                Some(why) => println!("coverage map {}: left out, {why}", c.map),
                None => println!(
                    "coverage map {}: {} edges from {} tests, {} from {} at {}{}",
                    c.map,
                    c.edges,
                    c.tests,
                    c.source,
                    c.created,
                    c.commit.get(..12).unwrap_or(&c.commit),
                    if c.ignored > 0 {
                        format!("; {} naming files no longer here", c.ignored)
                    } else {
                        String::new()
                    }
                ),
            }
        }
        println!(
            "workspace packages: {}, package edges: {}",
            s.packages, s.package_edges
        );
        println!(
            "unresolved: {}, unknown dynamic imports: {}, tsconfig fallbacks: {}",
            s.unresolved, s.unknown, s.tsconfig_fallbacks
        );
        if scan.rules.edges > 0
            || !scan.rules.unmatched.is_empty()
            || !scan.graph.barrier.is_empty()
        {
            println!(
                "rule edges: {}, barrier files: {}",
                scan.rules.edges,
                scan.graph.barrier.len()
            );
        }
        for from in &scan.rules.unmatched {
            println!("rule from {from} linked no file");
        }
        for (from, n) in &scan.rules.large {
            println!("rule from {from} added {n} edges; check that its globs match only what they should");
        }
        if scan.cache.enabled {
            println!(
                "parse cache: {} hits, {} parsed",
                scan.cache.hits, scan.cache.misses
            );
        } else {
            println!("parse cache: {cache_off}");
        }
        println!("built in {seconds:.2} s");
    }
    ExitCode::SUCCESS
}

fn lookup(scan: &Scan, file: &str) -> Option<u32> {
    let id = scan.graph.id(file);
    if id.is_none() {
        eprintln!("{file} isn't a file Fairlead can see (ignored, or outside the repository)");
    }
    id
}

fn why(scan: &Scan, from: &str, to: &str) -> ExitCode {
    let (Some(a), Some(b)) = (lookup(scan, from), lookup(scan, to)) else {
        return ExitCode::FAILURE;
    };
    match scan.graph.why(a, b) {
        Some(chain) => {
            // The planner walks from `to` towards `from` and stops at the first
            // barrier it meets, so any barrier but `from` itself cuts the chain.
            let stop = chain
                .iter()
                .skip(1)
                .rev()
                .find(|(f, _)| scan.graph.id(f).is_some_and(|id| scan.graph.is_barrier(id)));
            for (i, (file, kind)) in chain.iter().enumerate() {
                let via = kind
                    .map(|k| format!("  ({})", format!("{k:?}").to_lowercase()))
                    .unwrap_or_default();
                println!(
                    "{}{file}{}",
                    "  ".repeat(i),
                    if i + 1 < chain.len() {
                        via
                    } else {
                        String::new()
                    }
                );
            }
            if let Some((file, _)) = stop {
                println!(
                    "the test plan doesn't follow this: its walk stops at {file} (graph.barrier)"
                );
            }
            ExitCode::SUCCESS
        }
        None => {
            println!("{from} doesn't depend on {to}");
            ExitCode::FAILURE
        }
    }
}

fn importers(scan: &Scan, file: &str) -> ExitCode {
    let Some(id) = lookup(scan, file) else {
        return ExitCode::FAILURE;
    };
    for (importer, kind) in scan.graph.importers(id) {
        let kind = kind
            .map(|k| format!("{k:?}").to_lowercase())
            .unwrap_or_else(|| "package".into());
        println!("{}  ({kind})", scan.graph.files[importer as usize]);
    }
    ExitCode::SUCCESS
}
