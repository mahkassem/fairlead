//! `fairlead mcp`: an MCP server over stdio, newline-delimited JSON-RPC 2.0.
//! It serves read-only tools, one request at a time, and writes nothing to
//! stdout but protocol messages; each tool runs as a child process.

use std::io::{BufRead, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use fairlead_guard::events::{Event, EventLog};
use serde_json::{json, Value};

use crate::mcp_tools::{self, TOOLS};

/// Newest first; a client asking for one of these gets it back.
pub const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_LINE: usize = 1024 * 1024;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn initialize(params: &Value) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or("");
    let version = VERSIONS
        .iter()
        .find(|v| **v == asked)
        .unwrap_or(&VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "fairlead", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Fairlead plans the tests a change needs and briefs it: call `brief` with the paths you will touch before editing, and `receipt` or `next` before calling the change done.",
    })
}

fn list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.schema(),
                "annotations": { "readOnlyHint": true, "openWorldHint": false },
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// A tool's outcome: an error the agent can act on is a result with
/// `isError`, never a protocol error.
fn call(
    params: &Value,
    root: &Path,
) -> Result<(Value, &'static str, Option<String>), (i64, String)> {
    let name = params["name"].as_str().unwrap_or("");
    let tool = mcp_tools::find(name).ok_or((INVALID_PARAMS, format!("unknown tool `{name}`")))?;
    let input = params.get("arguments").cloned().unwrap_or(json!({}));
    if !input.is_object() {
        return Err((INVALID_PARAMS, "`arguments` is an object".into()));
    }
    let session = input["session"].as_str().map(str::to_string);
    let args = match tool.args(&input, root) {
        Ok(a) => a,
        Err(e) => return Ok((failed(&e), "refused", session)),
    };
    let ran = mcp_tools::run(&args, root);
    if !ran.ok {
        return Ok((failed(&ran.text), "error", session));
    }
    let mut text = ran.text;
    if ran.truncated {
        text.push_str(&format!("\n[truncated at {} bytes]", mcp_tools::MAX_OUTPUT));
    }
    let mut out = json!({ "content": [{ "type": "text", "text": text }], "isError": false });
    if !ran.truncated {
        if let Ok(parsed @ Value::Object(_)) = serde_json::from_str::<Value>(&text) {
            out["structuredContent"] = parsed;
        }
    }
    if tool.repository_text {
        out["_meta"] = json!({ "fairlead/repository-text": true });
    }
    Ok((out, "ok", session))
}

fn failed(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn record(
    root: &Path,
    tool: &str,
    decision: &'static str,
    session: Option<String>,
    started: Instant,
) {
    if let Some(log) = EventLog::open(root) {
        let mut event = Event::new("mcp", decision, started.elapsed());
        event.tool = Some(tool.to_string());
        event.session = session;
        let _ = log.append(&event);
    }
}

/// The reply to one line, if it needs one: notifications get none.
pub fn handle(line: &str, root: &Path) -> Option<Value> {
    if line.len() > MAX_LINE {
        return Some(error(&Value::Null, INVALID_REQUEST, "message over 1 MB"));
    }
    let message: Value = match serde_json::from_str(line) {
        Ok(m) => m,
        Err(_) => return Some(error(&Value::Null, PARSE_ERROR, "not JSON")),
    };
    if message.is_array() {
        return Some(error(
            &Value::Null,
            INVALID_REQUEST,
            "batches aren't supported",
        ));
    }
    let id = match message.get("id") {
        Some(id @ (Value::String(_) | Value::Number(_))) => id.clone(),
        Some(_) => {
            return Some(error(
                &Value::Null,
                INVALID_REQUEST,
                "`id` is a string or a number",
            ))
        }
        None => return None,
    };
    let Some(method) = message["method"].as_str() else {
        // A response to a request this server never sends.
        return None;
    };
    let params = message.get("params").cloned().unwrap_or(json!({}));
    Some(match method {
        "initialize" => result(&id, initialize(&params)),
        "ping" => result(&id, json!({})),
        "tools/list" => result(&id, list()),
        "tools/call" => {
            let started = Instant::now();
            let tool = params["name"].as_str().unwrap_or("").to_string();
            match call(&params, root) {
                Ok((out, decision, session)) => {
                    record(root, &tool, decision, session, started);
                    result(&id, out)
                }
                Err((code, message)) => error(&id, code, &message),
            }
        }
        _ => error(&id, METHOD_NOT_FOUND, &format!("no method `{method}`")),
    })
}

pub fn run(cwd: &Path) -> ExitCode {
    let root = crate::graph_cmd::repo_root(cwd);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(&line, &root) {
            let text = serde_json::to_string(&reply).expect("replies serialize");
            if writeln!(out, "{text}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
    ExitCode::SUCCESS
}
