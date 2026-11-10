//! `fairlead report` and the line each refusal ends with: the draft holds
//! numbers and Fairlead's own words, never a name the project chose, and
//! nothing is sent or left in the work tree.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A clean environment, so a developer's FAIRLEAD_*, CI or git settings
/// can't leak in.
fn command(dir: &Path, program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir).env_clear();
    if let Some(value) = std::env::var_os("SYSTEMROOT") {
        cmd.env("SYSTEMROOT", value);
    }
    cmd.env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "maintenance.auto")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "gc.auto")
        .env("GIT_CONFIG_VALUE_1", "0")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    cmd
}

/// The exit code, stdout and stderr of `fairlead` with `args`.
fn fairlead(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = command(dir, env!("CARGO_BIN_EXE_fairlead"), args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn git(dir: &Path, args: &[&str]) {
    let out = command(dir, "git", args).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Names a project would not want posted: a runner, a check, a need and a path.
const CONFIG: &str = r#"
[tests]
match = { replace = ["**/*.test.ts"] }

[[tests.runners]]
id = "acme-billing"
match = ["**/*.test.ts"]
command = ["sh", "-c", "exit 1", "sh", "{files}"]

[[checks]]
id = "acme-lint"
command = ["sh", "-c", "exit 0"]
paths = ["**/*.ts"]

[[needs]]
id = "acme-vault"
env = "ACME_VAULT_TOKEN"
tests = ["vault/**"]
proved_in = "the acme pipeline"
"#;

/// A repository named for a customer, with a change on a branch.
fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("acme-zzcustomer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/acme")).unwrap();
    std::fs::write(dir.join("fairlead.toml"), CONFIG).unwrap();
    std::fs::write(dir.join("src/acme/keys.ts"), "export const k = 1;\n").unwrap();
    let test = "import { k } from './keys';\n";
    std::fs::write(dir.join("src/acme/keys.test.ts"), test).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["checkout", "-q", "-b", "change"]);
    std::fs::write(dir.join("src/acme/keys.ts"), "export const k = 2;\n").unwrap();
    git(&dir, &["commit", "-qam", "change"]);
    dir
}

#[test]
fn a_failed_gate_ends_with_how_to_report_it_and_the_draft_names_nothing_of_the_project() {
    let dir = repo("gate");
    let (code, _, err) = fairlead(&dir, &["done"]);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("done: If this looks like Fairlead's mistake and not the change's, check it first (`fairlead guide report`), then `fairlead report` drafts a report for the project's owner to review and send."),
        "{err}"
    );
    assert!(err.contains("Never put a secret"), "{err}");

    let (code, out, err) = fairlead(&dir, &["report"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Nothing was sent."), "{out}");
    assert!(
        err.contains(
            "posts it at https://github.com/mahkassem/fairlead/issues/new?template=mistake.yml"
        ),
        "{err}"
    );
    assert!(
        err.contains("an agent posts it only when the owner said to in this session"),
        "{err}"
    );
    let draft = std::fs::read_to_string(dir.join(".git/fairlead/report.md")).unwrap();
    assert!(
        draft.contains(&format!("- fairlead {} on ", env!("CARGO_PKG_VERSION"))),
        "{draft}"
    );
    assert!(
        draft.contains("- config: full profile; runners 1, checks 1, needs 1, `[[graph.edges]]` rules 0; budget 300 s; phase not set"),
        "{draft}"
    );
    assert!(draft.contains("- setup (mvp track): binary "), "{draft}");
    assert!(draft.contains("done fail"), "{draft}");
    assert!(draft.contains("runner 1 failed in "), "{draft}");
    assert!(draft.contains("- [ ] The project's owner has read every line of this draft."));
    // Nothing the project named: not its folder, its ids, its paths or its need.
    for private in ["acme", "zzcustomer", "billing", "vault", "keys.", "ACME"] {
        assert!(
            !draft.contains(private),
            "`{private}` is in the draft:\n{draft}"
        );
    }
    // The draft is inside the git folder, so the work tree stays clean.
    let status = command(&dir, "git", &["status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout), "");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn print_writes_no_file_and_the_guide_says_what_never_goes_in() {
    let dir = repo("print");
    let (code, out, _) = fairlead(&dir, &["report", "--print"]);
    assert_eq!(code, 0);
    assert!(
        out.starts_with("<!-- A draft from `fairlead report`. Nothing has been sent. -->"),
        "{out}"
    );
    assert!(out.contains("- no refusal is on record here"), "{out}");
    assert!(!out.contains("acme"), "{out}");
    assert!(!dir.join(".git/fairlead/report.md").exists());

    let (code, guide, _) = fairlead(&dir, &["guide", "report"]);
    assert_eq!(code, 0);
    // The first line, whatever line ending the checkout gave the guide.
    assert_eq!(
        guide.lines().next(),
        Some("# Report a mistake of Fairlead's"),
        "{guide}"
    );
    for line in [
        "## First, check that it is Fairlead's mistake",
        "## What never goes in a report",
        "- any secret: a token, password, key, cookie or connection string",
        "An agent posts it only when the owner said to in this session",
    ] {
        assert!(guide.contains(line), "`{line}` is missing");
    }
    let (_, list, _) = fairlead(&dir, &["guide"]);
    assert!(list.contains("report"), "{list}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_commit_the_guard_stops_ends_with_how_to_report_it() {
    let dir = repo("commit");
    std::fs::write(dir.join(".env"), "TOKEN=1\n").unwrap();
    git(&dir, &["add", "-f", ".env"]);
    let (code, _, err) = fairlead(&dir, &["guard", "check", "--staged"]);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("guard: If this looks like Fairlead's mistake and not the change's"),
        "{err}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
