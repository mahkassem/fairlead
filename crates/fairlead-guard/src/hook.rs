//! The write stage's two ends: what a Claude Code or Codex PreToolUse call asks for,
//! read from its JSON, and the answer written back. Everything between is
//! the same engine the other stages use.

use serde_json::{json, Value};

use crate::edit::{self, Rebuilt, Replace};
use crate::patch::{self, FilePatch};

/// What the call would do, as far as the guard is concerned.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// A file's whole new content, from a write or a rebuilt edit.
    Write {
        path: String,
        text: String,
    },
    /// An edit to a file whose current text is needed to rebuild it.
    Edit {
        path: String,
        edits: Vec<(String, String, bool)>,
    },
    Bash {
        command: String,
    },
    /// A Codex `apply_patch`, which can touch several files at once.
    Patch {
        files: Vec<FilePatch>,
    },
    /// A call the guard reads but can't rebuild, with why.
    Unknown {
        tool: String,
        why: &'static str,
    },
    /// A tool the guard doesn't read.
    Ignored,
}

fn string<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

fn replace(edit: &Value) -> Option<(String, String, bool)> {
    Some((
        string(edit, "old_string")?.to_string(),
        string(edit, "new_string")?.to_string(),
        ["replace_all", "allow_multiple"]
            .iter()
            .any(|k| edit.get(*k).and_then(Value::as_bool) == Some(true)),
    ))
}

/// Reads the documented shapes: a whole `content` or `file_text`,
/// `old_string`/`new_string` edits, or a Codex patch in `command`. Any other
/// shape is unknown, never guessed.
pub fn request(call: &Value) -> Request {
    let tool = string(call, "tool_name").unwrap_or_default();
    let input = call.get("tool_input").unwrap_or(&Value::Null);
    let unknown = |why| Request::Unknown {
        tool: tool.to_string(),
        why,
    };
    // Gemini CLI names its tools in snake case, with Claude Code's arguments.
    let tool = match tool {
        "run_shell_command" => "Bash",
        "write_file" => "Write",
        "replace" => "Edit",
        other => other,
    };
    if tool == "Bash" {
        if let Some(files) = string(input, "command").and_then(shell_patch) {
            return Request::Patch { files };
        }
        return match string(input, "command") {
            Some(command) => Request::Bash {
                command: command.to_string(),
            },
            None => unknown("no command"),
        };
    }
    if tool == "apply_patch" {
        return match string(input, "command").map(patch::parse) {
            Some(Ok(files)) => Request::Patch { files },
            Some(Err(why)) => unknown(why),
            None => unknown("no command"),
        };
    }
    if !matches!(tool, "Write" | "Edit" | "MultiEdit") {
        return Request::Ignored;
    }
    let Some(path) = string(input, "file_path").map(String::from) else {
        return unknown("no file_path");
    };
    if let Some(text) = string(input, "content").or_else(|| string(input, "file_text")) {
        return Request::Write {
            path,
            text: text.to_string(),
        };
    }
    let edits: Option<Vec<_>> = match input.get("edits").and_then(Value::as_array) {
        Some(list) => list.iter().map(replace).collect(),
        None => replace(input).map(|e| vec![e]),
    };
    match edits {
        Some(edits) if !edits.is_empty() => Request::Edit { path, edits },
        _ => unknown("an edit shape the guard doesn't know"),
    }
}

/// A patch typed into the shell as `apply_patch <<'EOF'`, which Codex
/// applies as it would its own tool's.
fn shell_patch(command: &str) -> Option<Vec<FilePatch>> {
    let rest = command.trim_start();
    let rest = rest
        .strip_prefix("apply_patch")
        .or_else(|| rest.strip_prefix("applypatch"))?;
    let start = rest.find("*** Begin Patch")?;
    let end = rest.rfind("*** End Patch")? + "*** End Patch".len();
    rest[..start].trim().starts_with("<<").then_some(())?;
    patch::parse(rest.get(start..end)?).ok()
}

/// The file's text after the edits, applied to its current text.
pub fn rebuild(current: &str, edits: &[(String, String, bool)]) -> Rebuilt {
    let replaces: Vec<Replace<'_>> = edits
        .iter()
        .map(|(old, new, all)| Replace {
            old,
            new,
            all: *all,
        })
        .collect();
    edit::apply(current, &replaces)
}

/// The answer in the shape the calling agent reads: Gemini CLI's
/// `BeforeTool` takes a top-level `decision` and `reason`, and shows a
/// note only to the person, as `systemMessage`.
pub fn for_event(answer: &Value, event: Option<&str>) -> Value {
    if event != Some("BeforeTool") {
        return answer.clone();
    }
    let out = &answer["hookSpecificOutput"];
    match (
        out["permissionDecision"].as_str(),
        out["permissionDecisionReason"].as_str(),
    ) {
        (Some("deny"), Some(reason)) => json!({ "decision": "deny", "reason": reason }),
        _ => match out["additionalContext"].as_str() {
            Some(note) => json!({ "systemMessage": note }),
            None => answer.clone(),
        },
    }
}

/// Stops the call; the reason is what the agent reads.
pub fn deny(reason: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    })
}

/// Lets the call go on with a note the agent reads. It never says `allow`,
/// which would also skip the person's own permission prompt.
pub fn warn(context: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": context,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &str, input: Value) -> Value {
        json!({ "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input })
    }

    #[test]
    fn a_write_carries_its_whole_content_under_either_name() {
        let want = Request::Write {
            path: "/r/a.ts".into(),
            text: "x".into(),
        };
        assert_eq!(
            request(&call(
                "Write",
                json!({"file_path": "/r/a.ts", "content": "x"})
            )),
            want
        );
        assert_eq!(
            request(&call(
                "Write",
                json!({"file_path": "/r/a.ts", "file_text": "x"})
            )),
            want
        );
    }

    #[test]
    fn edit_and_multi_edit_carry_their_replacements() {
        let one = request(&call(
            "Edit",
            json!({"file_path": "a", "old_string": "o", "new_string": "n", "replace_all": true}),
        ));
        assert_eq!(
            one,
            Request::Edit {
                path: "a".into(),
                edits: vec![("o".into(), "n".into(), true)]
            }
        );
        let many = request(&call(
            "MultiEdit",
            json!({"file_path": "a", "edits": [
                {"old_string": "a", "new_string": "b"}, {"old_string": "b", "new_string": "c"}
            ]}),
        ));
        let Request::Edit { edits, .. } = many else {
            panic!("{many:?}")
        };
        assert_eq!(edits.len(), 2);
        assert_eq!(rebuild("a", &edits), Rebuilt::Text("c".into()));
    }

    #[test]
    fn an_unknown_shape_is_unknown_and_another_tool_is_ignored() {
        assert!(matches!(
            request(&call("Edit", json!({"file_path": "a", "diff": "x"}))),
            Request::Unknown { .. }
        ));
        assert!(matches!(
            request(&call(
                "MultiEdit",
                json!({"file_path": "a", "edits": [{"x": 1}]})
            )),
            Request::Unknown { .. }
        ));
        assert!(matches!(
            request(&call("Write", json!({"content": "x"}))),
            Request::Unknown { .. }
        ));
        assert_eq!(
            request(&call("Read", json!({"file_path": "a"}))),
            Request::Ignored
        );
        assert_eq!(request(&json!({})), Request::Ignored);
    }

    #[test]
    fn a_codex_patch_carries_its_files_and_a_broken_one_is_unknown() {
        let patch = "*** Begin Patch\n*** Add File: a.ts\n+x\n*** End Patch";
        let Request::Patch { files } = request(&call("apply_patch", json!({"command": patch})))
        else {
            panic!("not a patch")
        };
        assert_eq!(files[0].path, "a.ts");
        assert!(matches!(
            request(&call("apply_patch", json!({"command": "not a patch"}))),
            Request::Unknown { .. }
        ));
        assert!(matches!(
            request(&call("apply_patch", json!({}))),
            Request::Unknown { .. }
        ));
    }

    #[test]
    fn a_patch_typed_into_the_shell_is_a_patch_and_other_commands_stay_commands() {
        let typed =
            "apply_patch <<'EOF'\n*** Begin Patch\n*** Add File: a.ts\n+x\n*** End Patch\nEOF\n";
        assert!(matches!(
            request(&call("Bash", json!({"command": typed}))),
            Request::Patch { .. }
        ));
        let echo = "echo '*** Begin Patch' > notes.txt";
        assert!(matches!(
            request(&call("Bash", json!({"command": echo}))),
            Request::Bash { .. }
        ));
    }

    #[test]
    fn gemini_tools_read_as_their_claude_code_twins_and_answers_take_its_shape() {
        assert_eq!(
            request(&call(
                "write_file",
                json!({"file_path": "a.ts", "content": "x"})
            )),
            Request::Write {
                path: "a.ts".into(),
                text: "x".into()
            }
        );
        assert_eq!(
            request(&call(
                "replace",
                json!({"file_path": "a.ts", "old_string": "a", "new_string": "b", "allow_multiple": true})
            )),
            Request::Edit {
                path: "a.ts".into(),
                edits: vec![("a".into(), "b".into(), true)]
            }
        );
        assert!(matches!(
            request(&call("run_shell_command", json!({"command": "ls"}))),
            Request::Bash { .. }
        ));
        let denied = for_event(&deny("no"), Some("BeforeTool"));
        assert_eq!(denied, json!({"decision": "deny", "reason": "no"}));
        assert_eq!(
            for_event(&warn("note"), Some("BeforeTool")),
            json!({"systemMessage": "note"})
        );
        assert_eq!(for_event(&deny("no"), Some("PreToolUse")), deny("no"));
    }

    #[test]
    fn bash_carries_its_command() {
        assert_eq!(
            request(&call("Bash", json!({"command": "ls"}))),
            Request::Bash {
                command: "ls".into()
            }
        );
    }

    #[test]
    fn a_warning_never_grants_permission() {
        let out = warn("note");
        assert!(out["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none());
        assert_eq!(
            deny("why")["hookSpecificOutput"]["permissionDecision"],
            "deny"
        );
    }
}
