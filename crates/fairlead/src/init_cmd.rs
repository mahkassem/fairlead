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
    /// Replace a `fairlead.toml` that's already there, and a blueprint's files.
    #[arg(long)]
    force: bool,
    /// Also write a blueprint: a built-in (laravel-api, vite-react), or a
    /// blueprint file or folder of the team's own.
    #[arg(long, value_name = "NAME_OR_PATH")]
    blueprint: Option<String>,
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
    let blueprint = match args
        .blueprint
        .as_deref()
        .map(|b| crate::blueprint::load(b, cwd))
    {
        Some(Ok(bp)) => Some(bp),
        Some(Err(e)) => {
            eprintln!("fairlead: {e}");
            return ExitCode::from(2);
        }
        None => None,
    };
    let found = detect(&root, &files);
    if found.runners.is_empty() && found.checks.is_empty() {
        eprintln!("fairlead: found no test runner init knows here; the book's Configuration page shows how to add one");
        for note in &found.notes {
            eprintln!("  note: {note}");
        }
        return ExitCode::FAILURE;
    }
    let mut text = render(&found);
    if let Some(bp) = &blueprint {
        text = with_blueprint(&text, bp);
    }
    if args.dry_run {
        print!("{text}");
        for f in blueprint.iter().flat_map(|bp| &bp.files) {
            println!("# would write {}", f.path);
        }
        return ExitCode::SUCCESS;
    }
    // Checked beside the target before it replaces it, so a config that
    // doesn't load never lands; packs a blueprint names resolve from there.
    let draft = root.join(".fairlead.init.toml");
    if let Err(e) = std::fs::write(&draft, &text) {
        eprintln!("fairlead: could not write {}: {e}", draft.display());
        return ExitCode::from(2);
    }
    let checked = match config::load_file(&draft, &[]) {
        Ok(loaded) => config::validate(&loaded.config)
            .into_iter()
            .map(|p| format!("{}: {}", p.key, p.message))
            .collect::<Vec<_>>(),
        Err(e) => vec![format!("the config init made doesn't load: {e}")],
    };
    if !checked.is_empty() {
        let _ = std::fs::remove_file(&draft);
        for line in checked {
            eprintln!("fairlead: {line}");
        }
        return ExitCode::from(2);
    }
    if let Err(e) = std::fs::rename(&draft, &target) {
        let _ = std::fs::remove_file(&draft);
        eprintln!("fairlead: could not write {}: {e}", target.display());
        return ExitCode::from(2);
    }
    println!("fairlead: wrote {}", target.display());
    if let Some(bp) = &blueprint {
        if let Err(e) = write_blueprint(&root, bp, args.force) {
            eprintln!("fairlead: {e}");
            return ExitCode::from(2);
        }
    } else if let Some(bp) = crate::blueprint::suggest(&files) {
        println!(
            "  this looks like {}: `fairlead init --blueprint {} --force` also adds {}",
            bp.name, bp.name, bp.description
        );
    }
    summary(&found);
    ExitCode::SUCCESS
}

/// The detected config with a blueprint's: its top-level keys go before the
/// first table, where TOML reads them as top-level, and its tables at the end.
fn with_blueprint(text: &str, bp: &crate::blueprint::Blueprint) -> String {
    let lines: Vec<&str> = bp.config.trim().lines().collect();
    let split = lines
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .unwrap_or(lines.len());
    let top = lines[..split].join("\n");
    let tables = lines[split..].join("\n");
    let at = text.find("\n[").map_or(text.len(), |i| i + 1);
    let mut out = text[..at].to_string();
    if !top.trim().is_empty() {
        out.push_str(&format!(
            "# From the {} blueprint.\n{}\n\n",
            bp.name,
            top.trim()
        ));
    }
    out.push_str(&text[at..]);
    if !tables.trim().is_empty() {
        out.push_str(&format!(
            "\n# From the {} blueprint.\n{}\n",
            bp.name,
            tables.trim()
        ));
    }
    out
}

/// A blueprint's starter files, never over one that's there unless forced,
/// then its CI workflow.
fn write_blueprint(
    root: &Path,
    bp: &crate::blueprint::Blueprint,
    force: bool,
) -> Result<(), String> {
    for f in &bp.files {
        let path = root.join(&f.path);
        if path.exists() && !force {
            println!(
                "  kept {}: it's there already (--force replaces it)",
                f.path
            );
            continue;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        std::fs::write(&path, f.text.trim_start())
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        println!("  wrote {}", f.path);
    }
    if bp.ci_workflow {
        crate::ci_workflow::run(root, true, false)?;
    }
    Ok(())
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
    } else if depends("ava") {
        let matches = match manifest["ava"]["files"].as_array() {
            Some(globs) => globs
                .iter()
                .filter_map(|g| g.as_str())
                .map(String::from)
                .collect(),
            None => [
                "**/*.{test,spec}.{js,mjs,cjs,ts}",
                "**/test-*.{js,mjs,cjs,ts}",
                "test/**/*.{js,mjs,cjs,ts}",
                "tests/**/*.{js,mjs,cjs,ts}",
                "**/__tests__/**/*.{js,mjs,cjs,ts}",
            ]
            .map(String::from)
            .to_vec(),
        };
        found.notes.push(
            "ava: copy any options or environment (such as NODE_OPTIONS) from scripts.test into the runner's command"
                .into(),
        );
        (
            "ava",
            "ava in package.json",
            matches,
            with(&["ava", "{files}"]),
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
    let modules: &[&str] = match id {
        "vitest" => &["vitest"],
        "jest" => &["@jest/globals"],
        "bun" => &["bun:test"],
        "node" => &["node:test"],
        _ => &[],
    };
    let mut matches = matches;
    matches.extend(importing(root, files, &matches, modules));
    push(found, files, id, why, matches, command);
    if depends("@playwright/test") {
        found.notes.push(
            "Playwright specs are best planned in their own layer (`--env e2e`); see the book's Plans in CI page"
                .into(),
        );
    }
}

/// Patterns for test files the runner's patterns miss but that import its
/// module, such as a `test.ts` beside each function: a name two files share
/// becomes `**/name`, a name one file has stays that file's path.
fn importing(root: &Path, files: &[String], matches: &[String], modules: &[&str]) -> Vec<String> {
    if modules.is_empty() {
        return Vec::new();
    }
    let known: Vec<Pattern> = matches
        .iter()
        .filter_map(|g| Pattern::new(g).ok())
        .collect();
    let quoted: Vec<String> = modules
        .iter()
        .flat_map(|m| [format!("\"{m}\""), format!("'{m}'")])
        .collect();
    let mut names: std::collections::BTreeMap<&str, Vec<&String>> = Default::default();
    for file in files.iter().filter(|f| !f.contains("node_modules/")) {
        let name = file.rsplit('/').next().unwrap_or(file);
        let script = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"]
            .iter()
            .any(|e| name.ends_with(e));
        if !script || !(name.contains("test") || name.contains("spec")) {
            continue;
        }
        if known.iter().any(|p| p.is_match(file)) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        if quoted.iter().any(|q| text.contains(q.as_str())) {
            names.entry(name).or_default().push(file);
        }
    }
    names
        .into_iter()
        .flat_map(|(name, paths)| match paths.as_slice() {
            [one] => vec![(*one).clone()],
            _ => vec![format!("**/{name}")],
        })
        .collect()
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
    if has("tests/runtests.py") {
        found.notes.push(
            "tests/runtests.py is this project's own test runner; if its suite doesn't run under pytest, change the pytest runner's command"
                .into(),
        );
    }
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
    let matches =
        phpunit_dirs(root, files).unwrap_or_else(|| vec!["tests/**/*Test.php".to_string()]);
    push(found, files, id, why, matches, command);
}

/// The test directories `phpunit.xml` or `phpunit.xml.dist` names, each with
/// its `suffix` (`Test.php` by default), since not every project calls the
/// folder `tests`.
fn phpunit_dirs(root: &Path, files: &[String]) -> Option<Vec<String>> {
    let name = ["phpunit.xml", "phpunit.xml.dist"]
        .into_iter()
        .find(|n| files.iter().any(|f| f == n))?;
    let text = std::fs::read_to_string(root.join(name)).ok()?;
    let directory = regex::Regex::new(r#"<directory([^>]*)>\s*([^<]+?)\s*</directory>"#).ok()?;
    let suffix = regex::Regex::new(r#"suffix\s*=\s*"([^"]+)""#).ok()?;
    let mut out: Vec<String> = directory
        .captures_iter(&text)
        .map(|c| {
            let dir = c[2].trim_start_matches("./").trim_end_matches('/');
            let end = suffix
                .captures(&c[1])
                .map_or("Test.php".to_string(), |s| s[1].to_string());
            format!("{dir}/**/*{end}")
        })
        .collect();
    out.sort();
    out.dedup();
    (!out.is_empty()).then_some(out)
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
    if has("Gemfile")
        && !files.iter().any(|f| f.starts_with("spec/"))
        && files
            .iter()
            .any(|f| f.starts_with("test/") && f.ends_with("_test.rb"))
    {
        found.checks.push(Check {
            id: "rake-test",
            why: "Ruby isn't in the import graph yet",
            paths: vec!["**/*.rb", "Gemfile", "Gemfile.lock"],
            command: vec!["bundle", "exec", "rake", "test"],
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
    // Replacing the built-in patterns, which lists would otherwise append to,
    // keeps a test file no runner here covers out of the plan.
    let mut all: Vec<&String> = found.runners.iter().flat_map(|r| &r.matches).collect();
    all.sort();
    all.dedup();
    out.push_str(&format!(
        "\n[tests]\nmatch = {{ replace = {} }}\n",
        list(&all)
    ));
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
