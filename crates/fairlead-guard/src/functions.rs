//! A function's own lines: its first line to its last, minus the lines of the
//! functions directly nested in it, so a long callback is charged to itself
//! and not to every function around it.

use std::collections::{BTreeSet, HashSet};

use fairlead_lang::tree_sitter::{Node, Tree};

/// Every node kind that is a function, with a body or an expression.
const FUNCTIONS: [&str; 7] = [
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "function",
    "generator_function",
    "arrow_function",
    "method_definition",
];

pub(crate) struct Length {
    /// 1-based, where the function's first modifier or decorator is.
    pub line: u32,
    pub own: u64,
    pub total: u64,
    pub name: String,
}

/// Named, since the `function` keyword is a token of the same kind name.
fn is_function(node: Node<'_>) -> bool {
    node.is_named() && FUNCTIONS.contains(&node.kind())
}

/// The first line, counting an `export` in front of a declaration and the
/// decorators in front of a method, which belong to it.
fn first_row(node: Node<'_>) -> usize {
    if let Some(parent) = node.parent().filter(|p| p.kind() == "export_statement") {
        return parent.start_position().row;
    }
    let mut row = node.start_position().row;
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        match p.kind() {
            "decorator" => row = p.start_position().row,
            // A comment between a decorator and its method doesn't part them.
            "comment" => {}
            _ => break,
        }
        prev = p.prev_named_sibling();
    }
    row
}

/// The name of the callee's base identifier: `it` for `it(...)`,
/// `it.only(...)` and `it.each(...)(...)`.
fn callee_name<'a>(node: Node<'_>, text: &'a str) -> Option<&'a str> {
    match node.kind() {
        "identifier" => node.utf8_text(text.as_bytes()).ok(),
        "member_expression" => callee_name(node.child_by_field_name("object")?, text),
        "call_expression" => callee_name(node.child_by_field_name("function")?, text),
        _ => None,
    }
}

fn is_hook_callback(node: Node<'_>, text: &str, hooks: &HashSet<String>) -> bool {
    let Some(args) = node.parent().filter(|p| p.kind() == "arguments") else {
        return false;
    };
    args.parent()
        .filter(|call| call.kind() == "call_expression")
        .and_then(|call| call.child_by_field_name("function"))
        .and_then(|callee| callee_name(callee, text))
        .is_some_and(|name| hooks.contains(name))
}

fn name(node: Node<'_>, text: &str) -> String {
    let named = node.child_by_field_name("name").or_else(|| {
        let parent = node.parent()?;
        match parent.kind() {
            "variable_declarator" => parent.child_by_field_name("name"),
            "pair" => parent.child_by_field_name("key"),
            "public_field_definition" | "field_definition" => parent
                .child_by_field_name("name")
                .or_else(|| parent.child_by_field_name("property")),
            _ => None,
        }
    });
    named
        .and_then(|n| n.utf8_text(text.as_bytes()).ok())
        .unwrap_or_default()
        .to_string()
}

/// The rows the functions directly under `node` cover: the walk stops at
/// each function, so a deeper one is charged to its nearest enclosing one.
fn nested_rows(node: Node<'_>) -> BTreeSet<usize> {
    let mut rows = BTreeSet::new();
    let mut stack: Vec<Node<'_>> = children(node);
    while let Some(child) = stack.pop() {
        if is_function(child) {
            rows.extend(first_row(child)..=child.end_position().row);
        } else {
            stack.extend(children(child));
        }
    }
    rows
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

/// Every function in the tree except a callback passed straight to a test hook.
pub(crate) fn lengths(tree: &Tree, text: &str, hooks: &HashSet<String>) -> Vec<Length> {
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if is_function(node) && !is_hook_callback(node, text, hooks) {
            let (start, end) = (first_row(node), node.end_position().row);
            let total = (end - start + 1) as u64;
            out.push(Length {
                line: start as u32 + 1,
                own: total - nested_rows(node).len() as u64,
                total,
                name: name(node, text),
            });
        }
        stack.extend(children(node));
    }
    out.sort_by_key(|l| l.line);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lengths_of(path: &str, text: &str) -> Vec<(u32, u64, u64, String)> {
        let tree = fairlead_lang::extract::parse(path, text.as_bytes()).unwrap();
        let hooks: HashSet<String> = ["describe", "it", "test"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        lengths(&tree, text, &hooks)
            .into_iter()
            .map(|l| (l.line, l.own, l.total, l.name))
            .collect()
    }

    #[test]
    fn a_nested_function_is_charged_to_itself_not_its_parent() {
        let text = "function outer() {\n  const inner = () => {\n    a()\n  }\n  b()\n}\n";
        assert_eq!(
            lengths_of("a.ts", text),
            vec![(1, 3, 6, "outer".into()), (2, 3, 3, "inner".into())]
        );
    }

    #[test]
    fn export_and_decorators_start_the_function() {
        let text = "export function f() {\n}\nclass C {\n  @dec()\n  // why\n  m() {\n  }\n}\n";
        let found = lengths_of("a.ts", text);
        assert_eq!(found[0], (1, 2, 2, "f".into()));
        assert_eq!(found[1], (4, 4, 4, "m".into()));
    }

    #[test]
    fn a_test_hook_callback_is_not_measured_but_what_it_nests_is() {
        let text = "describe(\"x\", () => {\n  it.each([1])(\"y\", () => {\n    const f = function () {\n      a()\n    }\n  })\n})\n";
        assert_eq!(lengths_of("a.test.ts", text), vec![(3, 3, 3, "f".into())]);
    }

    #[test]
    fn methods_getters_constructors_and_generators_are_functions() {
        let text = "class C {\n  constructor() {}\n  get g() { return 1 }\n  *gen() {}\n}\nfunction* h() {}\n";
        let names: Vec<String> = lengths_of("a.ts", text).into_iter().map(|l| l.3).collect();
        assert_eq!(names, ["constructor", "g", "gen", "h"]);
    }
}
