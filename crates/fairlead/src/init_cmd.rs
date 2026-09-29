//! `fairlead init`: a first `fairlead.toml`, written from what the repository
//! shows (its manifests, lockfiles and test files) so a new user starts from a
//! config that plans and runs their tests instead of an empty one.

use std::path::Path;
use std::process::ExitCode;

use fairlead_core::config;
use fairlead_core::pattern::Pattern;
use serde_json::Value;

#[derive(clap::Args)]
pub struct InitArgs {
    /// Print the config instead of writing it.
    #[arg(long)]
    dry_run: bool,
    /// Replace a `fairlead.toml` that's already there.
    #[arg(long)]
    force: bool,
}

/// A test runner the repository shows, with the test files its `match` finds.
struct Runner {
    id: &'static str,
    why: String,
    matches: Vec<String>,
    command: Vec<String>,
    files: usize,
}

/// A language the graph doesn't read: any change to it runs the whole suite.
struct Check {
    id: &'static str,
    why: &'static str,
    paths: Vec<&'static str>,
    command: Vec<&'static str>,
}

struct Found {
    runners: Vec<Runner>,
    checks: Vec<Check>,
    notes: Vec<String>,
}

pub fn run(args: InitArgs, cwd: &Path) -> ExitCode {
    let root = crate::graph_cmd::repo_root(cwd);
    let target = root.join("fairlead.toml");
    if target.exists() && !args.force && !args.dry_run {
        eprintln!(
            "fairlead: {} already exists; `fairlead init --force` replaces it, `--dry-run` prints what init would write",
            target.display()
        );
        return ExitCode::from(2);
    }
    let files = fairlead_lang::tree::Tree::scan(&fairlead_lang::tree::plain(&root)).files;
    let found = detect(&root, &files);
    if found.runners.is_empty() && found.checks.is_empty() {
        eprintln!("fairlead: found no test runner init knows here; the book's Configuration page shows how to add one");
        for note in &found.notes {
            eprintln!("  note: {note}");
        }
        return ExitCode::FAILURE;
    }
    let text = render(&found);
    if args.dry_run {
        print!("{text}");
        return ExitCode::SUCCESS;
    }
    if let Err(e) = std::fs::write(&target, &text) {
        eprintln!("fairlead: could not write {}: {e}", target.display());
        return ExitCode::from(2);
    }
    match config::load_file(&target, &[]) {
        Ok(loaded) => {
            let problems = config::validate(&loaded.config);
            if !problems.is_empty() {
                for p in problems {
                    eprintln!("fairlead: {}: {}", p.key, p.message);
                }
                return ExitCode::from(2);
            }
        }
        Err(e) => {
            eprintln!("fairlead: the config init wrote doesn't load: {e}");
            return ExitCode::from(2);
        }
    }
    println!("fairlead: wrote {}", target.display());
    summary(&found);
    ExitCode::SUCCESS
}

fn summary(found: &Found) {
    for r in &found.runners {
        let plural = if r.files == 1 { "" } else { "s" };
        println!(
            "  {:<10} {:>4} test file{plural}  {}",
            r.id,
            r.files,
            r.command.join(" ")
        );
    }
    for c in &found.checks {
        println!("  {:<10} check     {}", c.id, c.command.join(" "));
    }
    for note in &found.notes {
        println!("  note: {note}");
    }
    println!("next:");
    println!("  fairlead plan --base main     what a change since main would run");
    println!("  fairlead hooks install        guard an agent's edits in Claude Code");
    println!("  fairlead brief <path>         what an edit will reach, before it's made");
}

fn detect(root: &Path, files: &[String]) -> Found {
    let mut found = Found {
        runners: Vec::new(),
        checks: Vec::new(),
        notes: Vec::new(),
    };
    let has = |name: &str| files.iter().any(|f| f == name);
    if has("package.json") {
        javascript(root, files, &mut found);
    }
    if has("go.mod") {
        let matches = vec!["**/*_test.go".to_string()];
        push(
            &mut found,
            files,
            "go",
            "go.mod",
            matches,
            args(&["go", "test", "{packages}"]),
        );
    } else if files.iter().any(|f| f.ends_with("/go.mod")) {
        found.notes.push(
            "a Go module below the root: add a runner per module with `invoke = \"per-module\"`"
                .into(),
        );
    }
    if [
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "pytest.ini",
        "tox.ini",
    ]
    .iter()
    .any(|f| has(f))
    {
        python(files, &mut found);
    }
    if has("composer.json") {
        php(root, files, &mut found);
    }
    fallbacks(files, &mut found);
    found
}

fn javascript(root: &Path, files: &[String], found: &mut Found) {
    let manifest: Value = std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let depends = |name: &str| {
        ["dependencies", "devDependencies"]
            .iter()
            .any(|k| manifest[k].get(name).is_some())
    };
    let resolved = test_scripts(&manifest);
    let script = resolved.as_str();
    let exec = exec_prefix(files);
    let standard = vec!["**/*.{test,spec}.{ts,tsx,js,jsx,mjs,cjs,mts,cts}".to_string()];
    let with = |tool: &[&str]| {
        let mut argv: Vec<String> = exec.iter().map(|s| s.to_string()).collect();
        argv.extend(tool.iter().map(|s| s.to_string()));
        argv
    };
    let (id, why, matches, command) = if depends("vitest") {
        (
            "vitest",
            "vitest in package.json",
            standard,
            with(&["vitest", "run", "{files}"]),
        )
    } else if depends("jest") {
        (
            "jest",
            "jest in package.json",
            standard,
            with(&["jest", "{files}"]),
        )
    } else if script.starts_with("bun test") {
        (
            "bun",
            "`bun test` in scripts.test",
            standard,
            args(&["bun", "test", "{files}"]),
        )
    } else if script.contains("node --test") {
        (
            "node",
            "`node --test` in scripts.test",
            standard,
            args(&["node", "--test", "{files}"]),
        )
    } else if depends("mocha") {
        let spec_dir = vec!["test/**/*.{js,mjs,cjs,ts}".to_string()];
        let matches = if count(files, &standard) > 0 {
            standard
        } else {
            spec_dir
        };
        found.notes.push(
            "mocha: copy any `--require` or reporter options from scripts.test into the runner's command"
                .into(),
        );
        (
            "mocha",
            "mocha in package.json",
            matches,
            with(&["mocha", "{files}"]),
        )
    } else {
        if !script.is_empty() {
            found.notes.push(format!(
                "the test scripts run `{script}`, which init doesn't recognise; add a [[tests.runners]] entry for it"
            ));
        }
        return;
    };
    push(found, files, id, why, matches, command);
    if depends("@playwright/test") {
        found.notes.push(
            "Playwright specs are best planned in their own layer (`--env e2e`); see the book's Plans in CI page"
                .into(),
        );
    }
}

/// `scripts.test`, with each script it runs by name (`npm run unit`) put in
/// that script's place, one level deep, since a runner is often one step down.
fn test_scripts(manifest: &Value) -> String {
    let scripts = &manifest["scripts"];
    let test = scripts["test"].as_str().unwrap_or("");
    let mut parts = Vec::new();
    for step in test.split("&&").map(str::trim).filter(|s| !s.is_empty()) {
        let words: Vec<&str> = step.split_whitespace().collect();
        let name = match words.as_slice() {
            ["npm" | "pnpm" | "bun", "run", name, ..] => Some(*name),
            ["yarn", name, ..] if *name != "run" => Some(*name),
            ["yarn", "run", name, ..] => Some(*name),
            _ => None,
        };
        match name.and_then(|n| scripts[n].as_str()) {
            Some(inner) => parts.push(inner.to_string()),
            None => parts.push(step.to_string()),
        }
    }
    parts.join(" && ")
}

/// How a JavaScript project runs a tool from its dependencies, told by its lockfile.
fn exec_prefix(files: &[String]) -> Vec<&'static str> {
    let has = |name: &str| files.iter().any(|f| f == name);
    if has("bun.lock") || has("bun.lockb") {
        vec!["bun", "x"]
    } else if has("pnpm-lock.yaml") {
        vec!["pnpm", "exec"]
    } else if has("yarn.lock") {
        vec!["yarn"]
    } else {
        vec!["npx"]
    }
}

fn python(files: &[String], found: &mut Found) {
    let has = |name: &str| files.iter().any(|f| f == name);
    let matches: Vec<String> = ["**/test_*.py", "**/*_test.py"]
        .iter()
        .map(|s| s.to_string())
        .filter(|glob| count(files, std::slice::from_ref(glob)) > 0)
        .collect();
    let (why, command) = if has("uv.lock") {
        ("uv.lock", args(&["uv", "run", "pytest", "{files}"]))
    } else if has("poetry.lock") {
        ("poetry.lock", args(&["poetry", "run", "pytest", "{files}"]))
    } else {
        (
            "a Python project file",
            args(&["python", "-m", "pytest", "{files}"]),
        )
    };
    push(found, files, "pytest", why, matches, command);
}

fn php(root: &Path, files: &[String], found: &mut Found) {
    let composer = std::fs::read_to_string(root.join("composer.json")).unwrap_or_default();
    let has = |name: &str| files.iter().any(|f| f == name);
    let (id, why, command) = if has("artisan") {
        (
            "artisan",
            "Laravel's artisan",
            args(&["php", "artisan", "test", "{files}"]),
        )
    } else if composer.contains("\"pestphp/pest\"") {
        (
            "pest",
            "pestphp/pest in composer.json",
            args(&["vendor/bin/pest", "{files}"]),
        )
    } else if has("phpunit.xml")
        || has("phpunit.xml.dist")
        || composer.contains("\"phpunit/phpunit\"")
    {
        (
            "phpunit",
            "PHPUnit's config",
            args(&["vendor/bin/phpunit", "{files}"]),
        )
    } else {
        return;
    };
    let matches = vec!["tests/**/*Test.php".to_string()];
    push(found, files, id, why, matches, command);
}

/// Languages the import graph doesn't read yet: a check runs the whole suite
/// on any change to them, so a change there still runs something.
fn fallbacks(files: &[String], found: &mut Found) {
    let has = |name: &str| files.iter().any(|f| f == name);
    if has("Cargo.toml") {
        found.checks.push(Check {
            id: "cargo-test",
            why: "Rust isn't in the import graph yet",
            paths: vec!["**/*.rs", "**/Cargo.toml", "Cargo.lock"],
            command: vec!["cargo", "test"],
        });
    }
    if has("gradlew") {
        found.checks.push(Check {
            id: "gradle-test",
            why: "Java and Kotlin aren't in the import graph yet",
            paths: vec!["**/*.java", "**/*.kt", "**/*.gradle", "**/*.gradle.kts"],
            command: vec!["./gradlew", "test"],
        });
    } else if has("pom.xml") {
        found.checks.push(Check {
            id: "maven-test",
            why: "Java isn't in the import graph yet",
            paths: vec!["**/*.java", "**/pom.xml"],
            command: vec!["mvn", "-q", "test"],
        });
    }
    if files
        .iter()
        .any(|f| f.ends_with(".sln") || f.ends_with(".csproj"))
    {
        found.checks.push(Check {
            id: "dotnet-test",
            why: ".NET isn't in the import graph yet",
            paths: vec!["**/*.cs", "**/*.csproj", "**/*.sln"],
            command: vec!["dotnet", "test"],
        });
    }
    if has("Gemfile") && files.iter().any(|f| f.starts_with("spec/")) {
        found.checks.push(Check {
            id: "rspec",
            why: "Ruby isn't in the import graph yet",
            paths: vec!["**/*.rb", "Gemfile", "Gemfile.lock"],
            command: vec!["bundle", "exec", "rspec"],
        });
    }
}

/// Adds the runner when its `match` finds a test file; otherwise says so.
fn push(
    found: &mut Found,
    files: &[String],
    id: &'static str,
    why: &str,
    matches: Vec<String>,
    command: Vec<String>,
) {
    let n = count(files, &matches);
    if n == 0 {
        found.notes.push(format!(
            "{id}: {why}, but no test file matches {}; add a runner once there are tests",
            if matches.is_empty() {
                "its patterns".into()
            } else {
                matches.join(", ")
            }
        ));
        return;
    }
    found.runners.push(Runner {
        id,
        why: why.to_string(),
        matches,
        command,
        files: n,
    });
}

fn count(files: &[String], globs: &[String]) -> usize {
    let patterns: Vec<Pattern> = globs.iter().filter_map(|g| Pattern::new(g).ok()).collect();
    files
        .iter()
        .filter(|f| !f.contains("node_modules/") && !f.starts_with("vendor/"))
        .filter(|f| patterns.iter().any(|p| p.is_match(f)))
        .count()
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn list<S: AsRef<str>>(items: &[S]) -> String {
    let quoted: Vec<String> = items
        .iter()
        .map(|s| {
            format!(
                "\"{}\"",
                s.as_ref().replace('\\', "\\\\").replace('"', "\\\"")
            )
        })
        .collect();
    format!("[{}]", quoted.join(", "))
}

/// The config, commented so a reader knows where each value came from.
fn render(found: &Found) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let minor = version.rsplit_once('.').map_or(version, |(m, _)| m);
    let mut out = format!(
        "# Written by `fairlead init` from what this repository shows. Edit freely:\n\
         # `fairlead config check` validates it, and the book's Configuration page\n\
         # lists every key.\n\
         fairlead = \"{minor}\"\n"
    );
    if !found.runners.is_empty() {
        let mut all: Vec<&String> = found.runners.iter().flat_map(|r| &r.matches).collect();
        all.sort();
        all.dedup();
        out.push_str(&format!("\n[tests]\nmatch = {}\n", list(&all)));
    }
    for r in &found.runners {
        out.push_str(&format!(
            "\n# {}: {} test files, from {}.\n[[tests.runners]]\nid = \"{}\"\nmatch = {}\ncommand = {}\n",
            r.id,
            r.files,
            r.why,
            r.id,
            list(&r.matches),
            list(&r.command)
        ));
    }
    for c in &found.checks {
        out.push_str(&format!(
            "\n# {}, so any change to it runs the whole suite.\n[[checks]]\nid = \"{}\"\npaths = {}\ncommand = {}\n",
            c.why,
            c.id,
            list(&c.paths),
            list(&c.command)
        ));
    }
    out
}
