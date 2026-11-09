//! `fairlead init --profile guard`: a config with only the guard, for a
//! stack the planner doesn't read. The stack is looked at only to pick
//! defaults, such as where migrations live; the git hook goes in through
//! lefthook, as `fairlead hooks install --git` puts it.

use std::path::Path;
use std::process::ExitCode;

use fairlead_core::config;
use fairlead_core::pattern::Pattern;

/// A migrations folder a framework keeps, and whether its file names start
/// with a sequence number that must be unique. Timestamps split by `_`, as
/// in `2024_01_31_000000_x.php`, would all share their year, so those get
/// only the rule that an existing migration doesn't change.
struct Stack {
    what: &'static str,
    glob: &'static str,
    numbered: bool,
}

const STACKS: [Stack; 5] = [
    Stack {
        what: "Laravel migrations",
        glob: "database/migrations/*.php",
        numbered: false,
    },
    Stack {
        what: "Rails migrations",
        glob: "db/migrate/*.rb",
        numbered: true,
    },
    Stack {
        what: "SQL migrations",
        glob: "migrations/*.sql",
        numbered: true,
    },
    Stack {
        what: "SQL migrations",
        glob: "db/migrations/*.sql",
        numbered: true,
    },
    Stack {
        what: "Prisma migrations",
        glob: "prisma/migrations/*/migration.sql",
        numbered: false,
    },
];

/// The stacks whose migrations the tree holds.
fn stacks(files: &[String]) -> Vec<&'static Stack> {
    STACKS
        .iter()
        .filter(|s| Pattern::new(s.glob).is_ok_and(|p| files.iter().any(|f| p.is_match(f))))
        .collect()
}

/// The branch changes merge into: the remote's default, else `origin/main`.
fn base(root: &Path) -> String {
    let out = std::process::Command::new("git")
        .args(["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"])
        .current_dir(root)
        .output();
    out.ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "origin/main".into())
}

pub fn render(root: &Path, files: &[String]) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let minor = version.rsplit_once('.').map_or(version, |(m, _)| m);
    let mut out = format!(
        "# Written by `fairlead init --profile guard`: the guard only, with no test\n\
         # planner. `fairlead config check` validates it, and the book's Guard page\n\
         # lists every rule.\n\
         fairlead = \"{minor}\"\n\
         profile = \"guard\"\n\
         \n\
         # A commit may not add a file named like a secret: keystores, keys, `.env`\n\
         # files. Already-tracked ones are warnings until `ratchet` is set.\n\
         [guard.paths]\n\
         # allow = [\"config/dev.key\"]\n\
         \n\
         # Secrets in what files say, found by gitleaks when it's installed.\n\
         [[guard.external]]\n\
         preset = \"gitleaks\"\n"
    );
    let base = base(root);
    // One `[guard.migrations]` table: the first stack found sets it.
    if let Some(s) = stacks(files).first() {
        out.push_str(&format!(
            "\n# {}: one that exists at {base} doesn't change.\n[guard.migrations]\nfiles = [\"{}\"]\nbase = \"{base}\"\n",
            s.what, s.glob
        ));
        if s.numbered {
            out.push_str(
                "# Numbers are unique, compared with the base fetched at a push.\nunique_prefix = {}\nfetch = true\n",
            );
        }
    }
    out
}

/// Writes the config, checked before it lands, then the git hook.
pub fn run(root: &Path, dry_run: bool) -> ExitCode {
    let files = fairlead_lang::tree::Tree::scan(&fairlead_lang::tree::plain(root)).files;
    let text = render(root, &files);
    if dry_run {
        print!("{text}");
        return ExitCode::SUCCESS;
    }
    let target = root.join("fairlead.toml");
    let draft = root.join(".fairlead.init.toml");
    if let Err(e) = std::fs::write(&draft, &text) {
        eprintln!("fairlead: could not write {}: {e}", draft.display());
        return ExitCode::from(2);
    }
    let checked = match config::load_file(&draft, &[]) {
        Ok(loaded) => config::validate(&loaded.config),
        Err(e) => {
            let _ = std::fs::remove_file(&draft);
            eprintln!("fairlead: the config init made doesn't load: {e}");
            return ExitCode::from(2);
        }
    };
    if !checked.is_empty() || std::fs::rename(&draft, &target).is_err() {
        let _ = std::fs::remove_file(&draft);
        for p in checked {
            eprintln!("fairlead: {}: {}", p.key, p.message);
        }
        return ExitCode::from(2);
    }
    println!("fairlead: wrote {} (guard-only profile)", target.display());
    let loaded = config::load_file(&target, &[])
        .map(|l| l.config)
        .unwrap_or_default();
    if let Err(e) = crate::hooks_cmd::install_git(root, &loaded) {
        eprintln!("fairlead: the git hook: {e}");
        return ExitCode::from(2);
    }
    if !on_path("gitleaks") {
        println!("  gitleaks isn't installed here; secret content isn't scanned until it is");
    }
    ExitCode::SUCCESS
}

pub fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|d| {
            d.join(name).is_file() || (cfg!(windows) && d.join(format!("{name}.exe")).is_file())
        })
    })
}
