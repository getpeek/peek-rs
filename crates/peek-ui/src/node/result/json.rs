//! A JSON value flattened into lines for display.
//!
//! Ported from `cell/JsonCell.tsx`. The reference renders this tree *inside the cell* and lets
//! the row grow to fit; `DataTable` virtualises with `uniform_list`, which needs every row the
//! same height, so the grid shows a one-line summary and the tree lives in the detail pane.
//!
//! Flattening to a list of lines rather than nesting elements keeps the rendering a plain loop
//! and makes the whole shape — indent, punctuation, truncation — testable without a window.

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

/// One rendered line of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Line {
    pub(super) depth: usize,
    /// `"key":` when this line sits inside an object.
    pub(super) key: Option<String>,
    pub(super) text: String,
    pub(super) token: Token,
    /// `123ch`, shown beside a middle-truncated string so its real length is still visible.
    pub(super) char_count: Option<usize>,
    /// Whether a comma follows, because this is not the last entry of its container.
    pub(super) comma: bool,
}

impl Line {
    fn new(depth: usize, key: Option<String>, text: String, token: Token, comma: bool) -> Self {
        Self {
            depth,
            key,
            text,
            token,
            char_count: None,
            comma,
        }
    }
}

/// Flattens `value` into lines.
///
/// A JSON column sometimes arrives as a *string* holding JSON — some drivers hand it back
/// unparsed — so a string that parses is rendered as the structure it describes rather than as
/// an escaped blob.
pub(super) fn lines(value: &Value) -> Vec<Line> {
    let parsed;
    let value = match value {
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(inner) if inner.is_object() || inner.is_array() => {
                parsed = inner;
                &parsed
            }
            _ => value,
        },
        other => other,
    };

    let mut out = Vec::new();
    push(value, 0, None, false, &mut out);
    out
}

fn push(value: &Value, depth: usize, key: Option<String>, comma: bool, out: &mut Vec<Line>) {
    match value {
        Value::Object(map) if map.is_empty() => {
            out.push(Line::new(depth, key, "{}".into(), Token::Brace, comma));
        }
        Value::Object(map) => {
            out.push(Line::new(depth, key, "{".into(), Token::Brace, false));
            let last = map.len() - 1;
            for (index, (name, child)) in map.iter().enumerate() {
                push(
                    child,
                    depth + 1,
                    Some(format!("\"{name}\":")),
                    index < last,
                    out,
                );
            }
            out.push(Line::new(depth, None, "}".into(), Token::Brace, comma));
        }
        Value::Array(items) if items.is_empty() => {
            out.push(Line::new(depth, key, "[]".into(), Token::Brace, comma));
        }
        Value::Array(items) => {
            out.push(Line::new(depth, key, "[".into(), Token::Brace, false));
            let last = items.len() - 1;
            for (index, child) in items.iter().enumerate() {
                push(child, depth + 1, None, index < last, out);
            }
            out.push(Line::new(depth, None, "]".into(), Token::Brace, comma));
        }
        Value::Null => out.push(Line::new(depth, key, "null".into(), Token::Null, comma)),
        Value::Bool(flag) => out.push(Line::new(depth, key, flag.to_string(), Token::Bool, comma)),
        Value::Number(number) => out.push(Line::new(
            depth,
            key,
            number.to_string(),
            Token::Number,
            comma,
        )),
        Value::String(text) => out.push(string_line(depth, key, text, comma)),
    }
}

fn string_line(depth: usize, key: Option<String>, text: &str, comma: bool) -> Line {
    if text.is_empty() {
        return Line::new(depth, key, "\"\"".into(), Token::Empty, comma);
    }
    let length = text.chars().count();
    if length <= LONG_STRING {
        return Line::new(depth, key, format!("\"{text}\""), Token::Text, comma);
    }
    let head: String = text.chars().take(HEAD).collect();
    let tail: String = text
        .chars()
        .skip(length.saturating_sub(TAIL))
        .collect::<String>();
    let mut line = Line::new(depth, key, format!("\"{head}…{tail}\""), Token::Text, comma);
    line.char_count = Some(length);
    line
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Token, lines};

    #[test]
    fn an_object_nests_its_entries_one_level_in() {
        let rendered = lines(&json!({"a": 1, "b": 2}));
        assert_eq!(rendered[0].text, "{");
        assert_eq!(rendered[1].depth, 1);
        assert_eq!(rendered[1].key.as_deref(), Some("\"a\":"));
        assert_eq!(rendered[1].text, "1");
        assert!(rendered[1].comma, "not the last entry");
        assert!(!rendered[2].comma, "the last entry takes no comma");
        assert_eq!(rendered.last().unwrap().text, "}");
    }

    #[test]
    fn empty_containers_render_on_one_line() {
        assert_eq!(lines(&json!({})).len(), 1);
        assert_eq!(lines(&json!({}))[0].text, "{}");
        assert_eq!(lines(&json!([]))[0].text, "[]");
    }

    #[test]
    fn arrays_have_no_keys() {
        let rendered = lines(&json!([1, 2]));
        assert!(rendered[1].key.is_none());
        assert_eq!(rendered[1].text, "1");
    }

    #[test]
    fn primitives_keep_their_kind_for_colouring() {
        assert_eq!(lines(&json!(null))[0].token, Token::Null);
        assert_eq!(lines(&json!(true))[0].token, Token::Bool);
        assert_eq!(lines(&json!(1.5))[0].token, Token::Number);
        assert_eq!(lines(&json!("x"))[0].token, Token::Text);
        assert_eq!(lines(&json!(""))[0].token, Token::Empty);
    }

    /// One access token must not push everything else off the pane.
    #[test]
    fn a_long_string_is_middle_truncated_with_its_length() {
        let long = "a".repeat(50);
        let line = &lines(&json!({ "token": long }))[1];
        assert_eq!(line.char_count, Some(50));
        assert!(line.text.contains('…'));
        assert!(line.text.starts_with("\"aaaaaaaaaaaaaa"), "{}", line.text);
    }

    #[test]
    fn a_string_at_the_threshold_is_left_whole() {
        let exact = "a".repeat(36);
        let line = &lines(&json!(exact))[0];
        assert_eq!(line.char_count, None);
        assert!(!line.text.contains('…'));
    }

    /// Truncation counts characters, not bytes, or it would slice a multi-byte character apart.
    #[test]
    fn truncation_is_character_wise() {
        let long = "é".repeat(50);
        let line = &lines(&json!(long))[0];
        assert_eq!(line.char_count, Some(50));
        assert!(line.text.chars().count() < 30);
    }

    /// Some drivers hand a JSON column back as a string; showing the structure beats showing an
    /// escaped blob.
    #[test]
    fn a_json_string_is_parsed_into_its_structure() {
        let rendered = lines(&json!(r#"{"a":1}"#));
        assert_eq!(rendered[0].text, "{");
        assert_eq!(rendered[1].key.as_deref(), Some("\"a\":"));
    }

    /// But a plain string that merely looks numeric stays a string.
    #[test]
    fn a_string_that_is_not_json_structure_stays_a_string() {
        let rendered = lines(&json!("42"));
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].token, Token::Text);
        assert_eq!(rendered[0].text, "\"42\"");
    }

    #[test]
    fn nesting_deepens_the_indent() {
        let rendered = lines(&json!({"a": {"b": [1]}}));
        let depths: Vec<usize> = rendered.iter().map(|line| line.depth).collect();
        assert_eq!(depths.iter().copied().max(), Some(3));
    }
}
