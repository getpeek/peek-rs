//! Filtering and ranking — the step Monaco performed in the reference app.
//!
//! `complete` answers a whole context: every table, or every in-scope column, plus the keywords
//! that could continue the clause. The Tauri app shipped that set to Monaco, whose suggest
//! controller matched it against the word under the cursor, scored it and ordered it.
//! gpui-component's completion menu renders the array it is handed, in order, so that work lives
//! here now.

use lsp_types::{CompletionItem, CompletionItemKind};

/// Beyond this the list is scroll-noise. Applied only while a prefix is being typed: an empty
/// prefix is the "show me the schema" case, which the reference leaves whole.
const MAX_MATCHES: usize = 50;

/// How well a label answers the typed prefix. Declaration order is rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchClass {
    /// The label is the prefix.
    Exact,
    /// The label starts with it.
    Prefix,
    /// It starts a word inside the label, so `id` finds `organisation_id`.
    WordPrefix,
    /// Its characters appear in order, scattered.
    Subsequence,
}

#[derive(Debug, Clone, Copy)]
struct Match {
    class: MatchClass,
    /// Bytes at the head of the *label* the prefix covers — what the menu paints, and always a
    /// char boundary of the label. Zero for anything that does not start the label.
    overlap: usize,
}

/// Drops what the typed prefix cannot mean, orders what survives, and records the matched leading
/// run in `filter_text`.
///
/// An empty prefix keeps every item and only orders it: the cursor has just moved past a `FROM`
/// and the whole schema is the answer.
pub(crate) fn rank(items: &mut Vec<CompletionItem>, prefix: &str) {
    if prefix.is_empty() {
        items.sort_by(|left, right| {
            kind_rank(left.kind)
                .cmp(&kind_rank(right.kind))
                .then_with(|| left.label.cmp(&right.label))
        });
        return;
    }

    let mut matched: Vec<(Match, CompletionItem)> = items
        .drain(..)
        .filter_map(|item| classify(&item.label, prefix).map(|found| (found, item)))
        .collect();

    // A better textual match always wins; kind only separates items that answer the prefix
    // equally well, and the shorter label wins after that because it is the more complete answer.
    matched.sort_by(|(left_match, left), (right_match, right)| {
        left_match
            .class
            .cmp(&right_match.class)
            .then_with(|| kind_rank(left.kind).cmp(&kind_rank(right.kind)))
            .then_with(|| left.label.len().cmp(&right.label.len()))
            .then_with(|| left.label.cmp(&right.label))
    });
    matched.truncate(MAX_MATCHES);

    items.extend(matched.into_iter().map(|(found, mut item)| {
        // The menu highlights `0..filter_text.len()` of the label, and falls back to the length
        // of its own trigger query when this is `None` — which blues characters that never
        // matched. An empty string is the only way to say "nothing at the head matched".
        item.filter_text = Some(item.label[..found.overlap].to_string());
        item
    }));
}

fn classify(label: &str, prefix: &str) -> Option<Match> {
    if let Some(overlap) = prefix_overlap(label, prefix) {
        let class = if overlap == label.len() {
            MatchClass::Exact
        } else {
            MatchClass::Prefix
        };
        return Some(Match { class, overlap });
    }

    if word_starts(label).any(|start| prefix_overlap(&label[start..], prefix).is_some()) {
        return Some(Match {
            class: MatchClass::WordPrefix,
            overlap: 0,
        });
    }

    is_subsequence(label, prefix).then_some(Match {
        class: MatchClass::Subsequence,
        overlap: 0,
    })
}

/// Bytes at the head of `label` that `prefix` covers, or `None` when the label does not start
/// with it.
///
/// Measured by walking the label rather than by taking `prefix.len()`: case folding can change a
/// character's byte length, and the result indexes the label.
fn prefix_overlap(label: &str, prefix: &str) -> Option<usize> {
    let mut chars = label.char_indices();
    for wanted in prefix.chars() {
        let (_, found) = chars.next()?;
        if !same_char(found, wanted) {
            return None;
        }
    }
    Some(chars.next().map_or(label.len(), |(index, _)| index))
}

/// Where a word begins inside `label`: after a separator, or at a lower-to-upper hop. This is
/// what lets `id` find `organisation_id` and `by` find `order by`.
fn word_starts(label: &str) -> impl Iterator<Item = usize> + '_ {
    let mut previous: Option<char> = None;
    label.char_indices().filter_map(move |(index, current)| {
        let begins = previous.is_some_and(|before| {
            matches!(before, '_' | '.' | ' ') || (before.is_lowercase() && current.is_uppercase())
        });
        previous = Some(current);
        begins.then_some(index)
    })
}

fn is_subsequence(label: &str, prefix: &str) -> bool {
    let mut wanted = prefix.chars();
    let mut next = wanted.next();
    for found in label.chars() {
        let Some(current) = next else {
            return true;
        };
        if same_char(found, current) {
            next = wanted.next();
        }
    }
    next.is_none()
}

fn same_char(left: char, right: char) -> bool {
    left == right || left.to_lowercase().eq(right.to_lowercase())
}

/// Smaller sorts first. One table rather than a per-context one: the cursor context has already
/// decided which kinds are in the list at all, so this only orders kinds that genuinely compete.
fn kind_rank(kind: Option<CompletionItemKind>) -> usize {
    const ORDER: [CompletionItemKind; 5] = [
        CompletionItemKind::SNIPPET,
        CompletionItemKind::VARIABLE,
        CompletionItemKind::FIELD,
        CompletionItemKind::CLASS,
        CompletionItemKind::KEYWORD,
    ];
    let Some(kind) = kind else {
        return ORDER.len();
    };
    ORDER
        .iter()
        .position(|known| *known == kind)
        .unwrap_or(ORDER.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, kind: CompletionItemKind) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            kind: Some(kind),
            ..Default::default()
        }
    }

    fn ranked(labels: &[(&str, CompletionItemKind)], prefix: &str) -> Vec<String> {
        let mut items: Vec<CompletionItem> = labels
            .iter()
            .map(|(label, kind)| item(label, *kind))
            .collect();
        rank(&mut items, prefix);
        items.into_iter().map(|item| item.label).collect()
    }

    const TABLE: CompletionItemKind = CompletionItemKind::CLASS;
    const COLUMN: CompletionItemKind = CompletionItemKind::FIELD;
    const KEYWORD: CompletionItemKind = CompletionItemKind::KEYWORD;

    #[test]
    fn a_prefix_drops_every_label_it_cannot_mean() {
        let order = ranked(&[("users", TABLE), ("organisations", TABLE)], "us");
        assert_eq!(order, ["users"]);
    }

    #[test]
    fn an_exact_match_leads() {
        let order = ranked(&[("idx", COLUMN), ("id", COLUMN)], "id");
        assert_eq!(order, ["id", "idx"]);
    }

    #[test]
    fn a_prefix_beats_a_word_prefix() {
        let order = ranked(&[("organisation_id", COLUMN), ("identity", COLUMN)], "id");
        assert_eq!(order, ["identity", "organisation_id"]);
    }

    #[test]
    fn a_word_prefix_beats_a_scattered_subsequence() {
        let order = ranked(&[("boundary", COLUMN), ("order by", KEYWORD)], "by");
        assert_eq!(order, ["order by", "boundary"]);
    }

    #[test]
    fn a_column_outranks_a_table_and_a_table_outranks_a_keyword() {
        let order = ranked(
            &[("using", KEYWORD), ("users", TABLE), ("usage", COLUMN)],
            "us",
        );
        assert_eq!(order, ["usage", "users", "using"]);
    }

    #[test]
    fn a_shorter_label_wins_a_tie() {
        let order = ranked(&[("user_sessions", TABLE), ("users", TABLE)], "user");
        assert_eq!(order, ["users", "user_sessions"]);
    }

    #[test]
    fn matching_ignores_case_and_the_highlight_follows_the_label() {
        let mut items = vec![item("users", TABLE)];
        rank(&mut items, "US");
        assert_eq!(items[0].filter_text.as_deref(), Some("us"));
    }

    #[test]
    fn a_non_leading_match_claims_no_highlight() {
        let mut items = vec![item("organisation_id", COLUMN)];
        rank(&mut items, "id");
        assert_eq!(items[0].filter_text.as_deref(), Some(""));
    }

    #[test]
    fn an_overlap_never_splits_a_multibyte_label() {
        let mut items = vec![item("ärende_id", COLUMN)];
        rank(&mut items, "Ä");
        assert_eq!(items[0].filter_text.as_deref(), Some("ä"));
    }

    #[test]
    fn the_join_snippet_leads_an_unfiltered_list() {
        let order = ranked(
            &[
                ("id", COLUMN),
                ("users", TABLE),
                ("u.organisation_id = o.id", CompletionItemKind::SNIPPET),
            ],
            "",
        );
        assert_eq!(order[0], "u.organisation_id = o.id");
    }

    #[test]
    fn an_empty_prefix_keeps_every_item_and_orders_it_by_kind_then_label() {
        let order = ranked(
            &[
                ("where", KEYWORD),
                ("users", TABLE),
                ("name", COLUMN),
                ("id", COLUMN),
            ],
            "",
        );
        assert_eq!(order, ["id", "name", "users", "where"]);
    }

    #[test]
    fn a_long_list_is_capped_but_an_empty_prefix_is_not() {
        let labels: Vec<String> = (0..60).map(|index| format!("a_table_{index:02}")).collect();
        let build =
            || -> Vec<CompletionItem> { labels.iter().map(|label| item(label, TABLE)).collect() };

        let mut typed = build();
        rank(&mut typed, "a");
        assert_eq!(typed.len(), MAX_MATCHES);

        let mut untouched = build();
        rank(&mut untouched, "");
        assert_eq!(untouched.len(), 60);
    }
}
