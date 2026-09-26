//! The write stage's two ends: what a Claude Code PreToolUse call asks for,
//! read from its JSON, and the answer written back. Everything between is
//! the same engine the other stages use.

use serde_json::{json, Value};

use crate::edit::{self, Rebuilt, Replace};

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
        edit.get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ))
}

/// Reads the documented shapes: a whole `content` or `file_text`, or
/// `old_string`/`new_string` edits. Any other shape is unknown, never guessed.
pub fn request(call: &Value) -> Request {
    let tool = string(call, "tool_name").unwrap_or_default();
    let input = call.get("tool_input").unwrap_or(&Value::Null);
    let unknown = |why| Request::Unknown {
        tool: tool.to_string(),
        why,
    };
    if tool == "Bash" {
        return match string(input, "command") {
            Some(command) => Request::Bash {
                command: command.to_string(),
            },
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
