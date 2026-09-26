//! The write-hook budget from issue #3: p95 under 50 ms on a generated
//! history of TypeScript edits, through the hook binary, process start
//! included. Ignored by default because it only means something in a
//! release build: CI runs it with `--release -- --ignored`.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

const COMMITS: usize = 40;
const FILES: usize = 8;
const P95_UNDER_MS: &str = "50";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A module of `functions` functions, each with a comment block, so both
/// presets and the parse have real work.
fn module(seed: usize, functions: usize) -> String {
    let mut src = String::from("/**\n * A generated module.\n */\nimport { x } from \"./x\"\n\n");
    for f in 0..functions {
        let _ = write!(
            src,
            "// Keeps the order stable, since callers compare results.\nexport function f{seed}_{f}(a: number, b: string) {{\n  const c = a * {f}\n  if (b.length > c) {{\n    return b.slice(0, c)\n  }}\n  return `${{b}}-${{c}}`\n}}\n\n"
        );
    }
    src
}

#[test]
#[ignore]
fn the_write_hook_keeps_its_p95_under_the_budget() {
    let dir = std::env::temp_dir().join(format!("fairlead-hook-perf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(
        dir.join("fairlead.toml"),
        concat!(
            "[guard]\nbudget_ms = 10000\n",
            "[guard.size]\nfiles = [\"src/**\"]\nfile_lines = 1000\nfunction_lines = 120\n",
            "[guard.comments]\nfiles = [\"src/**\"]\nblock_length = { source = 8, header = 12, inline = 2 }\n",
            "density = { source = 0.25 }\nhistory = { dates = true, phrases = [\"used to\", \"no longer\"] }\n",
            "item_codes = { pattern = '(?-u:\\b)[A-Z]+-[0-9]+(?-u:\\b)', pointer = true }\n",
            "agent_phrases = ['(?i)(?-u:\\b)you must(?-u:\\b)']\nblock_marker = true\n",
        ),
    )
    .unwrap();
    for f in 0..FILES {
        std::fs::write(dir.join(format!("src/m{f}.ts")), module(f, 30 + f * 12)).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "start"]);
    let start = git(&dir, &["rev-parse", "HEAD"]);
    for c in 0..COMMITS {
        let f = c % FILES;
        std::fs::write(
            dir.join(format!("src/m{f}.ts")),
            module(f, 30 + f * 12 + c + 1),
        )
        .unwrap();
        git(&dir, &["commit", "-qam", &format!("edit {c}")]);
    }
    let out = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .args([
            "guard",
            "bench",
            "--since",
            &start,
            "--p95-under",
            P95_UNDER_MS,
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("{text}");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{text}");
    assert!(text.contains(&format!("{COMMITS} edit(s)")), "{text}");
}
