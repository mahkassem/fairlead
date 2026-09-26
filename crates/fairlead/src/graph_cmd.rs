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
fn repo_root(start: &Path) -> PathBuf {
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
        GraphAction::Stats { json } => stats(&scan, elapsed.as_secs_f64(), json),
        GraphAction::Why { from, to } => why(
            &scan,
            &relative_to(&root, cwd, &from),
            &relative_to(&root, cwd, &to),
        ),
        GraphAction::Importers { file } => importers(&scan, &relative_to(&root, cwd, &file)),
    }
}

fn stats(scan: &Scan, seconds: f64, json: bool) -> ExitCode {
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
            "cache": { "enabled": scan.cache.enabled, "hits": scan.cache.hits, "misses": scan.cache.misses },
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
        println!(
            "workspace packages: {}, package edges: {}",
            s.packages, s.package_edges
        );
        println!(
            "unresolved: {}, unknown dynamic imports: {}, tsconfig fallbacks: {}",
            s.unresolved, s.unknown, s.tsconfig_fallbacks
        );
        if scan.cache.enabled {
            println!(
                "parse cache: {} hits, {} parsed",
                scan.cache.hits, scan.cache.misses
            );
        } else {
            println!("parse cache: off");
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
