//! The one line a JSON cell shows in the grid.
//!
//! The reference renders a whole tree inside the cell and lets the row grow; a virtualised table
//! needs every row the same height, so the cell gets a preview and the tree opens in the pane.
//! `{…} 3 keys` was the first answer and it says nothing about the value — two rows with the same
//! shape and completely different contents read identically. This shows the content instead.
//!
//! **Bounded by construction.** At most [`ENTRIES`] entries and [`BUDGET`] characters, and a
//! nested container renders as `{…}` rather than recursing, so the cost of a preview is the same
//! for a four-key object and a four-megabyte one. That is what makes it safe on the render path,
//! where it runs once per visible JSON cell per frame, and why it is not cached: the walk is
//! cheaper than the `Value::to_string` the cell used to do.

use serde_json::Value;

use super::structure;

/// How many entries of a container are shown before `…`.
const ENTRIES: usize = 4;
/// Roughly how wide the preview is allowed to get. Not exact: an entry is never cut mid-way,
/// because half a value reads as a different value.
const BUDGET: usize = 64;
/// A string inside the preview is cut harder than one in the tree — there are several on a line.
const STRING: usize = 18;

/// `{ id: 42, user: {…}, tags: […] }`, or the scalar itself.
pub(in crate::node::result) fn preview(value: &Value) -> String {
    let parsed = structure(value);
    match parsed.as_ref().unwrap_or(value) {
        Value::Object(map) if map.is_empty() => "{}".to_string(),
        Value::Array(items) if items.is_empty() => "[]".to_string(),
        Value::Object(map) => wrap(
            ('{', '}'),
            map.iter()
                .map(|(key, child)| format!("{key}: {}", inner(child))),
            map.len(),
        ),
        Value::Array(items) => wrap(('[', ']'), items.iter().map(inner), items.len()),
        other => inner(other),
    }
}

/// Joins as many entries as fit inside [`BUDGET`], closing with `…` when any were left out.
fn wrap(braces: (char, char), entries: impl Iterator<Item = String>, total: usize) -> String {
    let (open, close) = braces;
    let mut shown = Vec::new();
    let mut width = 0;
    for entry in entries.take(ENTRIES) {
        // Always take the first, so a single very long entry previews as itself truncated
        // rather than as an empty `{ … }` that says nothing at all.
        if !shown.is_empty() && width + entry.len() > BUDGET {
            break;
        }
        width += entry.len() + 2;
        shown.push(entry);
    }
    let elided = if shown.len() < total { ", …" } else { "" };
    format!("{open} {}{elided} {close}", shown.join(", "))
}

/// One value inside the preview: a container collapses to its braces, so the walk never
/// recurses and the cost cannot depend on how deep the document goes.
fn inner(value: &Value) -> String {
    match value {
        Value::Object(map) if map.is_empty() => "{}".to_string(),
        Value::Array(items) if items.is_empty() => "[]".to_string(),
        Value::Object(_) => "{…}".to_string(),
        Value::Array(_) => "[…]".to_string(),
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => {
            let length = text.chars().count();
            if length <= STRING {
                format!("\"{text}\"")
            } else {
                let head: String = text.chars().take(STRING - 1).collect();
                format!("\"{head}…\"")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{BUDGET, preview};

    #[test]
    fn an_object_shows_its_first_entries() {
        assert_eq!(
            preview(&json!({"id": 42, "name": "ada"})),
            "{ id: 42, name: \"ada\" }"
        );
    }

    #[test]
    fn empty_containers_keep_their_old_rendering() {
        assert_eq!(preview(&json!({})), "{}");
        assert_eq!(preview(&json!([])), "[]");
    }

    #[test]
    fn a_scalar_in_a_json_column_shows_as_itself() {
        assert_eq!(preview(&json!(7)), "7");
        assert_eq!(preview(&json!(true)), "true");
        assert_eq!(preview(&json!(null)), "null");
    }

    /// The whole point of bounding it: a preview of a huge document costs the same as a preview
    /// of a small one, because it never looks past the first few entries.
    #[test]
    fn a_preview_is_bounded_however_large_the_value() {
        let big: serde_json::Map<String, serde_json::Value> = (0..10_000)
            .map(|index| (format!("key_{index:05}"), json!(index)))
            .collect();
        let rendered = preview(&serde_json::Value::Object(big));
        assert!(rendered.len() < BUDGET * 2, "{rendered}");
        assert!(rendered.ends_with(", … }"), "{rendered}");
    }

    /// A nested container is a shape, not a recursion — otherwise depth would drive the cost.
    #[test]
    fn nested_containers_do_not_recurse() {
        assert_eq!(
            preview(&json!({"a": {"b": {"c": 1}}, "d": [1, 2]})),
            "{ a: {…}, d: […] }"
        );
    }

    #[test]
    fn a_long_string_is_cut_inside_the_preview() {
        let rendered = preview(&json!({ "token": "a".repeat(200) }));
        assert!(rendered.contains('…'));
        assert!(rendered.len() < BUDGET * 2, "{rendered}");
    }

    /// One oversized entry still previews as itself rather than as an empty pair of braces,
    /// which would tell the reader nothing at all.
    #[test]
    fn a_single_oversized_entry_is_still_shown() {
        let rendered = preview(&json!({ "a_very_long_key_name_indeed": "x".repeat(200) }));
        assert!(
            rendered.contains("a_very_long_key_name_indeed"),
            "{rendered}"
        );
    }

    #[test]
    fn an_array_previews_its_items() {
        assert_eq!(preview(&json!([1, 2, 3])), "[ 1, 2, 3 ]");
        assert_eq!(preview(&json!([1, 2, 3, 4, 5])), "[ 1, 2, 3, 4, … ]");
    }

    /// A column handed back as a string still previews as the structure it holds.
    #[test]
    fn a_json_string_previews_as_its_structure() {
        assert_eq!(preview(&json!(r#"{"a":1}"#)), "{ a: 1 }");
    }
}
