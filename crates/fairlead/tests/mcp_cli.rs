//! `fairlead mcp` over stdio: lines in, lines out. Each test feeds a
//! conversation and reads every line the server wrote, so a stray print
//! or a reply to a notification shows up as a line that isn't expected.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fairlead-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write(&dir, "fairlead.toml", "[[tests.runners]]\nid = \"vitest\"\nmatch = [\"test/**\"]\ncommand = [\"vitest\", \"run\", \"{files}\"]\n");
    write(&dir, "src/a.ts", "export const a = 1;\n");
    write(&dir, "test/a.test.ts", "import { a } from '../src/a';\n");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// Every line the server wrote for `input`, parsed.
fn converse(dir: &Path, input: &[String]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .arg("mcp")
        .current_dir(dir)
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in input {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|_| panic!("not JSON on stdout: {l}")))
        .collect()
}

fn msg(v: Value) -> String {
    v.to_string()
}

#[test]
fn the_handshake_echoes_a_known_version_and_notifications_get_no_reply() {
    let dir = repo("handshake");
    let replies = converse(
        &dir,
        &[
            msg(
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
            ),
            msg(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})),
            msg(
                json!({"jsonrpc": "2.0", "id": "a", "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}),
            ),
            msg(
                json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}),
            ),
            msg(json!({"jsonrpc": "2.0", "id": 2, "method": "ping"})),
        ],
    );
    assert_eq!(
        replies.len(),
        3,
        "one reply per request, none per notification: {replies:?}"
    );
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(
        replies[0]["result"]["capabilities"],
        json!({"tools": {"listChanged": false}})
    );
    assert_eq!(replies[1]["id"], "a", "a string id comes back a string");
    assert_eq!(
        replies[1]["result"]["protocolVersion"], "2025-06-18",
        "an unknown version gets the newest"
    );
    assert_eq!(replies[2]["result"], json!({}));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tools_list_their_schemas_and_a_call_returns_text_and_structured_content() {
    let dir = repo("call");
    write(&dir, "src/a.ts", "export const a = 2;\n");
    let replies = converse(
        &dir,
        &[
            msg(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})),
            msg(
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "plan", "arguments": {"base": "main"}}}),
            ),
            msg(
                json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "explain", "arguments": {"target": "test/a.test.ts", "base": "main"}}}),
            ),
        ],
    );
    let tools = replies[0]["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(
        names.contains(&"plan") && names.contains(&"brief") && !names.contains(&"done"),
        "{names:?}"
    );
    let plan = &replies[1]["result"];
    assert_eq!(plan["isError"], false);
    assert_eq!(plan["content"][0]["type"], "text");
    assert_eq!(
        plan["structuredContent"]["tests"][0]["path"],
        "test/a.test.ts"
    );
    let text = plan["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap(),
        plan["structuredContent"]
    );
    let explain = &replies[2]["result"];
    assert_eq!(explain["isError"], false);
    assert!(
        explain.get("structuredContent").is_none(),
        "text tools carry no structured content"
    );
    assert!(explain["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("src/a.ts"));
    let log = std::fs::read_to_string(dir.join(".git/fairlead/events.jsonl")).unwrap();
    assert_eq!(log.matches("\"stage\":\"mcp\"").count(), 2, "{log}");
    assert!(
        !log.contains("test/a.test.ts"),
        "arguments never reach the log"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_input_gets_the_right_error_and_never_escapes_the_repository() {
    let dir = repo("errors");
    let replies = converse(
        &dir,
        &[
            "{not json".to_string(),
            msg(json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}])),
            msg(json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"})),
            msg(
                json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "done"}}),
            ),
            msg(
                json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "brief", "arguments": {"paths": ["../../etc/passwd"]}}}),
            ),
            msg(
                json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "plan", "arguments": {"base": "--output=/tmp/x"}}}),
            ),
            msg(
                json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "brief", "arguments": {"paths": ["/etc/passwd"]}}}),
            ),
        ],
    );
    let code = |i: usize| replies[i]["error"]["code"].as_i64();
    assert_eq!(code(0), Some(-32700));
    assert_eq!(code(1), Some(-32600), "batches are gone from the protocol");
    assert_eq!(
        code(2),
        Some(-32601),
        "an unoffered capability is an unknown method"
    );
    assert_eq!(code(3), Some(-32602), "done isn't served");
    for i in [4, 5, 6] {
        assert_eq!(replies[i]["result"]["isError"], true, "{}", replies[i]);
    }
    assert!(replies[4]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("outside the repository"));
    assert!(replies[5]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("not starting with -"));
    let _ = std::fs::remove_dir_all(&dir);
}
