//! Rendering a JSON value: the tree the value pane draws, and the one-line preview a grid cell
//! shows.
//!
//! Ported from `cell/JsonCell.tsx`, which renders the tree *inside the cell* and lets the row
//! grow to fit. `DataTable` virtualises with `uniform_list`, which needs every row the same
//! height, so the two halves split: [`preview`] gives the grid one line, and [`tree`] gives the
//! pane a foldable, searchable view of the whole value.
//!
//! What is shared between them lives here: the token kinds both colour by, and the middle
//! truncation both apply, so a string that reads `"aaaa…aaa"` in a cell reads the same in the
//! tree.

mod preview;
mod tree;

pub(super) use preview::preview;
pub(super) use tree::{Line, View};

use serde_json::Value;

/// Strings longer than this are middle-truncated, so one token or URL cannot dominate the view.
const LONG_STRING: usize = 36;
const HEAD: usize = 14;
const TAIL: usize = 6;

/// What a piece of a JSON value is, so the view can colour it without re-inspecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Token {
    /// `{`, `}`, `[`, `]`
    Brace,
    Null,
    Bool,
    Number,
    Text,
    /// An empty string, which reads as nothing at all without its own treatment.
    Empty,
}

/// A JSON column sometimes arrives as a *string* holding JSON — some drivers hand it back
/// unparsed — so a string that parses is treated as the structure it describes rather than as an
/// escaped blob. A string that merely looks numeric stays a string.
pub(super) fn structure(value: &Value) -> Option<Value> {
    let Value::String(text) = value else {
        return None;
    };
    match serde_json::from_str::<Value>(text) {
        Ok(inner) if inner.is_object() || inner.is_array() => Some(inner),
        _ => None,
    }
}

/// A string as it is displayed: quoted, and middle-truncated with its real length when it is
/// long enough to crowd out everything beside it.
fn quoted(text: &str) -> (String, Option<usize>, Token) {
    if text.is_empty() {
        return ("\"\"".to_string(), None, Token::Empty);
    }
    let length = text.chars().count();
    if length <= LONG_STRING {
        return (format!("\"{text}\""), None, Token::Text);
    }
    let head: String = text.chars().take(HEAD).collect();
    let tail: String = text.chars().skip(length - TAIL).collect();
    (format!("\"{head}…{tail}\""), Some(length), Token::Text)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Token, quoted, structure};

    /// One access token must not push everything else off the pane.
    #[test]
    fn a_long_string_is_middle_truncated_with_its_length() {
        let (text, count, token) = quoted(&"a".repeat(50));
        assert_eq!(count, Some(50));
        assert_eq!(token, Token::Text);
        assert!(text.contains('…'));
        assert!(text.starts_with("\"aaaaaaaaaaaaaa"), "{text}");
    }

    #[test]
    fn a_string_at_the_threshold_is_left_whole() {
        let (text, count, _) = quoted(&"a".repeat(36));
        assert_eq!(count, None);
        assert!(!text.contains('…'));
    }

    /// Truncation counts characters, not bytes, or it would slice a multi-byte character apart.
    #[test]
    fn truncation_is_character_wise() {
        let (text, count, _) = quoted(&"é".repeat(50));
        assert_eq!(count, Some(50));
        assert!(text.chars().count() < 30);
    }

    #[test]
    fn an_empty_string_keeps_its_own_kind() {
        let (text, _, token) = quoted("");
        assert_eq!(text, "\"\"");
        assert_eq!(token, Token::Empty);
    }

    /// Some drivers hand a JSON column back as a string; showing the structure beats showing an
    /// escaped blob.
    #[test]
    fn a_json_string_is_recognised_as_structure() {
        assert_eq!(structure(&json!(r#"{"a":1}"#)), Some(json!({"a": 1})));
        assert_eq!(structure(&json!("[1,2]")), Some(json!([1, 2])));
    }

    /// But a plain string that merely looks numeric stays a string, and so does a JSON scalar:
    /// unwrapping `"42"` to a number would silently change what the column holds.
    #[test]
    fn a_string_that_is_not_json_structure_stays_a_string() {
        assert_eq!(structure(&json!("42")), None);
        assert_eq!(structure(&json!("hello")), None);
        assert_eq!(structure(&json!("true")), None);
    }

    #[test]
    fn a_value_that_is_not_a_string_is_left_alone() {
        assert_eq!(structure(&json!({"a": 1})), None);
        assert_eq!(structure(&json!(7)), None);
    }
}
