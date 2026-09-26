//! What a file will hold after an agent's edit, rebuilt from the tool's
//! input, so the write stage can lint the file before the edit happens. An
//! edit that can't be rebuilt exactly is reported as such, never guessed at.

/// One replacement: `old` must occur in the text, once unless `all`.
pub struct Replace<'a> {
    pub old: &'a str,
    pub new: &'a str,
    pub all: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rebuilt {
    Text(String),
    /// Why the edit can't be rebuilt, such as an `old` that isn't there.
    Unknown(&'static str),
}

/// Applies each replacement in order, as the editing tools do, or says
/// which one it couldn't apply.
pub fn apply(text: &str, edits: &[Replace<'_>]) -> Rebuilt {
    let mut text = text.to_string();
    for edit in edits {
        if edit.old.is_empty() {
            return Rebuilt::Unknown("an edit replaces an empty string");
        }
        let count = text.matches(edit.old).count();
        text = match (count, edit.all) {
            (0, _) => return Rebuilt::Unknown("an edit's old text isn't in the file"),
            (1, _) | (_, true) => text.replace(edit.old, edit.new),
            _ => return Rebuilt::Unknown("an edit's old text occurs more than once"),
        };
    }
    Rebuilt::Text(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r<'a>(old: &'a str, new: &'a str, all: bool) -> Replace<'a> {
        Replace { old, new, all }
    }

    #[test]
    fn edits_apply_in_order_and_replace_all_replaces_every_one() {
        let text = "a b a\n";
        assert_eq!(
            apply(text, &[r("a", "c", true), r("b", "d", false)]),
            Rebuilt::Text("c d c\n".into())
        );
        assert_eq!(
            apply(text, &[r("b", "a", false), r("a a a", "x", false)]),
            Rebuilt::Text("x\n".into())
        );
    }

    #[test]
    fn an_edit_that_would_fail_in_the_tool_is_unknown_not_guessed() {
        assert_eq!(
            apply("a a", &[r("a", "b", false)]),
            Rebuilt::Unknown("an edit's old text occurs more than once")
        );
        assert_eq!(
            apply("a", &[r("z", "b", false)]),
            Rebuilt::Unknown("an edit's old text isn't in the file")
        );
        assert_eq!(
            apply("a", &[r("", "b", false)]),
            Rebuilt::Unknown("an edit replaces an empty string")
        );
    }
}
