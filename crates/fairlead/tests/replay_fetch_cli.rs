//! `fairlead replay fetch` and its token: with none at all a refusal says so,
//! and the one `gh auth token` prints is used when the environment has none.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-fetch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An API that answers 404 to everything, as GitHub does for a private
/// repository it is asked about without a token that can read it.
fn not_found() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                    break;
                }
            }
            let mut stream = stream;
            let body = r#"{"message":"Not Found"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 404 X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

fn fetch(dir: &Path, api: &str, path: &Path) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fairlead"));
    cmd.args([
        "replay",
        "fetch",
        "--repo",
        "o/r",
        "--data",
        "runs.jsonl",
        "--since",
        "2026-01-01",
    ])
    .current_dir(dir)
    .env_clear()
    .env("PATH", path)
    .env("HOME", dir)
    .env("GITHUB_API_URL", api);
    if let Some(root) = std::env::var_os("SYSTEMROOT") {
        cmd.env("SYSTEMROOT", root);
    }
    let out = cmd.output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

#[cfg(unix)]
#[test]
fn a_refusal_with_no_token_says_so_and_a_token_from_gh_is_used() {
    use std::os::unix::fs::PermissionsExt;
    let dir = dir("token");
    let api = not_found();
    // A PATH that holds curl and nothing named `gh`.
    let tools = dir.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let curl = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("curl"))
        .find(|p| p.exists())
        .expect("curl on the PATH");
    std::os::unix::fs::symlink(curl, tools.join("curl")).unwrap();
    let (code, out) = fetch(&dir, &api, &tools);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("GitHub answered 404"), "{out}");
    assert!(
        out.contains(
            "no token was sent: set GITHUB_TOKEN or GH_TOKEN, or sign in with `gh auth login`"
        ),
        "{out}"
    );

    // With a `gh` that prints a token, the same refusal is the repository's, not a missing token's.
    let gh = tools.join("gh");
    std::fs::write(&gh, "#!/bin/sh\necho token-from-gh\n").unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (code, out) = fetch(&dir, &api, &tools);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("GitHub answered 404"), "{out}");
    assert!(!out.contains("no token was sent"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
