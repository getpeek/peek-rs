//! A JSON value as a foldable tree.
//!
//! Flat, not nested. Every node of the value becomes one entry in a `Vec` in document order, so
//! a container's descendants are the contiguous run between it and its closing brace — which
//! makes "skip a folded subtree" an index jump instead of a walk, and makes the whole shape
//! testable without a window.
//!
//! **The visible-line list is rebuilt whole on every fold and every search, and never per
//! frame.** For a hundred thousand nodes that rebuild is one linear pass over a `Vec<usize>`,
//! which is far below what anyone can perceive on a keypress; splicing it incrementally would be
//! more code to defend for no difference anybody could see. What matters is only that it is not
//! on the render path, and it is not: `View::line` borrows and allocates nothing.

use std::collections::HashSet;

use gpui_kit::SharedString;
use serde_json::Value;

use super::{Token, quoted, structure};

/// One entry of the arena.
struct Node {
    depth: usize,
    /// `"name":` when this entry sits inside an object.
    key: Option<SharedString>,
    /// The value, or the opening brace of a container.
    text: SharedString,
    token: Token,
    /// `123ch`, shown beside a middle-truncated string so its real length is still visible.
    char_count: Option<usize>,
    /// Whether a comma follows. On a container's *opening* entry this is the comma the closing
    /// brace carries, so a folded one-line container still punctuates correctly.
    comma: bool,
    container: Option<Container>,
}

struct Container {
    /// One past this container's closing brace, so `open + 1 .. end` is the whole subtree.
    end: usize,
    /// What the entry shows when folded: `{…} 3 keys`. The grid used to show this for every
    /// JSON cell; now that a cell previews its contents, the wording survives here, where a
    /// folded subtree really does have nothing to say but its shape.
    summary: SharedString,
}

/// One line as the pane draws it. Borrowed from the arena: rendering allocates nothing.
pub(in crate::node::result) struct Line<'a> {
    pub(in crate::node::result) arena: usize,
    pub(in crate::node::result) depth: usize,
    pub(in crate::node::result) key: Option<&'a str>,
    pub(in crate::node::result) text: &'a str,
    pub(in crate::node::result) token: Token,
    pub(in crate::node::result) char_count: Option<usize>,
    pub(in crate::node::result) comma: bool,
    /// `Some(folded)` when this line opens a container the reader can fold; `None` otherwise.
    pub(in crate::node::result) fold: Option<bool>,
    pub(in crate::node::result) matched: bool,
}

/// A parsed value, plus what the reader has folded and searched for.
pub(in crate::node::result) struct View {
    nodes: Vec<Node>,
    folded: HashSet<usize>,
    /// Arena indexes in display order. Rebuilt by [`View::rebuild`] on a fold or a search.
    visible: Vec<usize>,
    matches: HashSet<usize>,
    searching: bool,
}

impl std::fmt::Debug for View {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("View")
            .field("nodes", &self.nodes.len())
            .field("visible", &self.visible.len())
            .field("folded", &self.folded.len())
            .finish_non_exhaustive()
    }
}

impl View {
    /// Parses `value`, fully expanded — the state the reference renders, and the one that shows
    /// what is in a value before asking the reader to open anything.
    pub(in crate::node::result) fn open(value: &Value) -> Self {
        let parsed = structure(value);
        let mut nodes = Vec::new();
        push(parsed.as_ref().unwrap_or(value), 0, None, false, &mut nodes);
        let mut view = Self {
            nodes,
            folded: HashSet::new(),
            visible: Vec::new(),
            matches: HashSet::new(),
            searching: false,
        };
        view.rebuild();
        view
    }

    pub(in crate::node::result) fn visible(&self) -> usize {
        self.visible.len()
    }

    /// Whether anything here can be folded at all, so the pane can leave out controls that would
    /// do nothing — a scalar in a JSON column has no structure to open or close.
    pub(in crate::node::result) fn foldable(&self) -> bool {
        self.nodes.iter().any(|node| node.container.is_some())
    }

    pub(in crate::node::result) fn matches(&self) -> usize {
        self.matches.len()
    }

    pub(in crate::node::result) fn is_searching(&self) -> bool {
        self.searching
    }

    /// The line at `position` in display order.
    pub(in crate::node::result) fn line(&self, position: usize) -> Option<Line<'_>> {
        let arena = *self.visible.get(position)?;
        let node = self.nodes.get(arena)?;
        let folded = self.folded.contains(&arena);
        let container = node.container.as_ref();
        let (text, char_count) = match container {
            Some(container) if folded => (container.summary.as_ref(), None),
            _ => (node.text.as_ref(), node.char_count),
        };
        Some(Line {
            arena,
            depth: node.depth,
            key: node.key.as_deref(),
            text,
            token: node.token,
            char_count,
            // An expanded container's opening brace takes no comma; its closing one does.
            comma: node.comma && (container.is_none() || folded),
            fold: container.map(|_| folded),
            matched: self.matches.contains(&arena),
        })
    }

    /// Folds or unfolds the container at `arena`, doing nothing for a leaf.
    pub(in crate::node::result) fn toggle(&mut self, arena: usize) {
        if self
            .nodes
            .get(arena)
            .is_none_or(|node| node.container.is_none())
        {
            return;
        }
        if !self.folded.remove(&arena) {
            self.folded.insert(arena);
        }
        self.rebuild();
    }

    /// Folds or unfolds everything. Folding all still leaves the outermost line, which is what
    /// keeps the pane from going blank.
    pub(in crate::node::result) fn set_all_folded(&mut self, folded: bool) {
        self.folded.clear();
        if folded {
            self.folded.extend(
                self.nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| node.container.is_some())
                    .map(|(index, _)| index),
            );
        }
        self.rebuild();
    }

    /// Marks the lines holding `query` and opens whatever was hiding them.
    ///
    /// Plain case-insensitive containment, not the fuzzy scorer the result grid uses: inside one
    /// value you already know what you are looking for, and a subsequence match would light up
    /// most lines of any large document. Non-matches stay visible — the pane dims them — because
    /// a key means little without the structure around it.
    pub(in crate::node::result) fn search(&mut self, query: &str) {
        self.matches.clear();
        self.searching = !query.trim().is_empty();
        if !self.searching {
            self.rebuild();
            return;
        }
        let needle = query.trim().to_lowercase();
        for (index, node) in self.nodes.iter().enumerate() {
            let hit = node.text.to_lowercase().contains(&needle)
                || node
                    .key
                    .as_ref()
                    .is_some_and(|key| key.to_lowercase().contains(&needle));
            if hit {
                self.matches.insert(index);
            }
        }
        self.unfold_ancestors_of_matches();
        self.rebuild();
    }

    /// Opens every container a match sits inside, so a hit is never hidden behind a fold.
    ///
    /// One pass with a stack of the containers currently open, rather than testing every
    /// container against every match.
    fn unfold_ancestors_of_matches(&mut self) {
        let mut open: Vec<usize> = Vec::new();
        for index in 0..self.nodes.len() {
            while open.last().is_some_and(|ancestor| {
                self.nodes[*ancestor]
                    .container
                    .as_ref()
                    .is_some_and(|container| container.end <= index)
            }) {
                open.pop();
            }
            if self.matches.contains(&index) {
                for ancestor in &open {
                    self.folded.remove(ancestor);
                }
            }
            if self.nodes[index].container.is_some() {
                open.push(index);
            }
        }
    }

    /// Walks the arena, skipping the subtree of anything folded.
    fn rebuild(&mut self) {
        self.visible.clear();
        let mut index = 0;
        while index < self.nodes.len() {
            self.visible.push(index);
            match &self.nodes[index].container {
                Some(container) if self.folded.contains(&index) => index = container.end,
                _ => index += 1,
            }
        }
    }
}

/// Appends `value` and its descendants, returning nothing: the arena is the output.
fn push(value: &Value, depth: usize, key: Option<String>, comma: bool, out: &mut Vec<Node>) {
    match value {
        Value::Object(map) if map.is_empty() => {
            out.push(leaf(depth, key, "{}", Token::Brace, comma));
        }
        Value::Array(items) if items.is_empty() => {
            out.push(leaf(depth, key, "[]", Token::Brace, comma));
        }
        Value::Object(map) => {
            let open = out.len();
            out.push(leaf(depth, key, "{", Token::Brace, comma));
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
            out.push(leaf(depth, None, "}", Token::Brace, comma));
            close(out, open, summary("{…}", map.len(), "key", "keys"));
        }
        Value::Array(items) => {
            let open = out.len();
            out.push(leaf(depth, key, "[", Token::Brace, comma));
            let last = items.len() - 1;
            for (index, child) in items.iter().enumerate() {
                push(child, depth + 1, None, index < last, out);
            }
            out.push(leaf(depth, None, "]", Token::Brace, comma));
            close(out, open, summary("[…]", items.len(), "item", "items"));
        }
        Value::Null => out.push(leaf(depth, key, "null", Token::Null, comma)),
        Value::Bool(flag) => out.push(leaf(depth, key, flag.to_string(), Token::Bool, comma)),
        Value::Number(number) => {
            out.push(leaf(depth, key, number.to_string(), Token::Number, comma));
        }
        Value::String(text) => {
            let (rendered, char_count, token) = quoted(text);
            let mut node = leaf(depth, key, rendered, token, comma);
            node.char_count = char_count;
            out.push(node);
        }
    }
}

fn leaf(
    depth: usize,
    key: Option<String>,
    text: impl Into<SharedString>,
    token: Token,
    comma: bool,
) -> Node {
    Node {
        depth,
        key: key.map(SharedString::from),
        text: text.into(),
        token,
        char_count: None,
        comma,
        container: None,
    }
}

/// Turns the entry at `open` into a container now that its subtree has been written.
fn close(out: &mut [Node], open: usize, summary: String) {
    let end = out.len();
    out[open].container = Some(Container {
        end,
        summary: SharedString::from(summary),
    });
}

fn summary(braces: &str, count: usize, singular: &str, plural: &str) -> String {
    let unit = if count == 1 { singular } else { plural };
    format!("{braces} {count} {unit}")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Token, View};

    /// The lines a fully expanded view draws, as `"<indent><key> <text><comma>"`.
    fn rendered(view: &View) -> Vec<String> {
        (0..view.visible())
            .map(|position| {
                let line = view.line(position).expect("inside the visible range");
                format!(
                    "{}{}{}{}",
                    "  ".repeat(line.depth),
                    line.key.map(|key| format!("{key} ")).unwrap_or_default(),
                    line.text,
                    if line.comma { "," } else { "" }
                )
            })
            .collect()
    }

    #[test]
    fn an_object_nests_its_entries_one_level_in() {
        let view = View::open(&json!({"a": 1, "b": 2}));
        assert_eq!(rendered(&view), vec!["{", "  \"a\": 1,", "  \"b\": 2", "}"]);
    }

    #[test]
    fn empty_containers_render_on_one_line() {
        assert_eq!(rendered(&View::open(&json!({}))), vec!["{}"]);
        assert_eq!(rendered(&View::open(&json!([]))), vec!["[]"]);
    }

    #[test]
    fn arrays_have_no_keys() {
        let view = View::open(&json!([1, 2]));
        assert!(view.line(1).unwrap().key.is_none());
        assert_eq!(view.line(1).unwrap().text, "1");
    }

    #[test]
    fn primitives_keep_their_kind_for_colouring() {
        assert_eq!(View::open(&json!(null)).line(0).unwrap().token, Token::Null);
        assert_eq!(View::open(&json!(true)).line(0).unwrap().token, Token::Bool);
        assert_eq!(
            View::open(&json!(1.5)).line(0).unwrap().token,
            Token::Number
        );
        assert_eq!(View::open(&json!("x")).line(0).unwrap().token, Token::Text);
        assert_eq!(View::open(&json!("")).line(0).unwrap().token, Token::Empty);
    }

    #[test]
    fn nesting_deepens_the_indent() {
        let view = View::open(&json!({"a": {"b": [1]}}));
        let deepest = (0..view.visible())
            .map(|position| view.line(position).unwrap().depth)
            .max();
        assert_eq!(deepest, Some(3));
    }

    /// A JSON column that arrived as a string still opens as the structure it describes.
    #[test]
    fn a_json_string_is_parsed_into_its_structure() {
        let view = View::open(&json!(r#"{"a":1}"#));
        assert_eq!(view.line(0).unwrap().text, "{");
        assert_eq!(view.line(1).unwrap().key, Some("\"a\":"));
    }

    #[test]
    fn folding_a_container_hides_its_subtree_and_summarises_it() {
        let mut view = View::open(&json!({"a": {"b": 1, "c": 2}, "d": 3}));
        let opened = view.visible();
        view.toggle(1);
        assert!(view.visible() < opened, "the subtree went away");
        assert_eq!(
            rendered(&view),
            vec!["{", "  \"a\": {…} 2 keys,", "  \"d\": 3", "}"],
            "a folded container keeps the comma its closing brace carried"
        );
    }

    #[test]
    fn folding_is_a_toggle() {
        let mut view = View::open(&json!({"a": {"b": 1}}));
        let opened = rendered(&view);
        view.toggle(1);
        view.toggle(1);
        assert_eq!(rendered(&view), opened);
    }

    #[test]
    fn folding_a_leaf_does_nothing() {
        let mut view = View::open(&json!({"a": 1}));
        let before = rendered(&view);
        view.toggle(1);
        assert_eq!(rendered(&view), before);
    }

    /// Folding everything must still leave the outermost line, or the pane goes blank and the
    /// reader has nothing left to click to get back.
    #[test]
    fn fold_all_leaves_the_root_and_expand_all_restores_every_line() {
        let value = json!({"a": {"b": [1, 2]}, "c": {"d": 4}});
        let mut view = View::open(&value);
        let opened = rendered(&view);

        view.set_all_folded(true);
        assert_eq!(view.visible(), 1, "only the root survives");
        assert_eq!(view.line(0).unwrap().text, "{…} 2 keys");

        view.set_all_folded(false);
        assert_eq!(rendered(&view), opened);
    }

    #[test]
    fn a_scalar_offers_nothing_to_fold() {
        assert!(!View::open(&json!("hello")).foldable());
        assert!(View::open(&json!({"a": 1})).foldable());
    }

    #[test]
    fn search_marks_the_lines_holding_the_query() {
        let mut view = View::open(&json!({"name": "ada", "city": "london"}));
        view.search("ada");
        assert_eq!(view.matches(), 1);
        assert!(view.is_searching());
        let marked: Vec<&str> = (0..view.visible())
            .filter_map(|position| view.line(position))
            .filter(|line| line.matched)
            .map(|line| line.text)
            .collect();
        assert_eq!(marked, vec!["\"ada\""]);
    }

    #[test]
    fn search_matches_keys_as_well_as_values() {
        let mut view = View::open(&json!({"email": "a@b.c"}));
        view.search("EMAIL");
        assert_eq!(view.matches(), 1, "and it ignores case");
    }

    /// A hit behind a fold is a hit nobody can see.
    #[test]
    fn search_opens_the_containers_a_match_is_hiding_in() {
        let mut view = View::open(&json!({"outer": {"inner": {"needle": 1}}}));
        view.set_all_folded(true);
        assert_eq!(view.visible(), 1);

        view.search("needle");
        let texts: Vec<&str> = (0..view.visible())
            .filter_map(|position| view.line(position))
            .map(|line| line.text)
            .collect();
        assert!(texts.contains(&"1"), "the match is on screen: {texts:?}");
    }

    /// Structure around a match is what makes it readable, so nothing is hidden — the pane dims
    /// the rest instead.
    #[test]
    fn search_leaves_non_matching_lines_visible() {
        let mut view = View::open(&json!({"a": 1, "b": 2}));
        let opened = view.visible();
        view.search("a");
        assert_eq!(view.visible(), opened);
    }

    #[test]
    fn clearing_the_query_stops_searching() {
        let mut view = View::open(&json!({"a": 1}));
        view.search("a");
        view.search("   ");
        assert!(!view.is_searching());
        assert_eq!(view.matches(), 0);
    }

    /// Reading a line must not change what the next read sees.
    #[test]
    fn reading_lines_does_not_disturb_the_view() {
        let view = View::open(&json!({"a": [1, 2, 3]}));
        let first = rendered(&view);
        let second = rendered(&view);
        assert_eq!(first, second);
        assert!(view.line(view.visible()).is_none(), "past the end is None");
    }

    #[test]
    fn a_long_string_carries_its_length_into_the_line() {
        let view = View::open(&json!({ "token": "a".repeat(50) }));
        let line = view.line(1).unwrap();
        assert_eq!(line.char_count, Some(50));
        assert!(line.text.contains('…'));
    }

    /// A folded container shows its summary, not a truncated-string badge from the entry it
    /// replaced.
    #[test]
    fn a_folded_container_shows_no_character_count() {
        let mut view = View::open(&json!({ "a": { "token": "a".repeat(50) } }));
        view.toggle(1);
        assert_eq!(view.line(1).unwrap().char_count, None);
    }
}
