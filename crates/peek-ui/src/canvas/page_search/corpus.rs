//! What page search matches against: `~/labs/peek/src/page-search/searchCorpus.ts` and
//! `useNodeSearch.ts`.
//!
//! One [`Entry`] per searchable node on the active page, built from what the node says —
//! [`peek_canvas::describe`], which the AI grouping prompt reads too. What this file adds is
//! the scoring: a result node's *title* is its SQL, which is already the query node's job to
//! match, so results match on their data alone.

use peek_canvas::describe;
use peek_document::{Node, NodeId, NodeType, ResultSet};

use crate::fuzzy::{MATCH_THRESHOLD, Match, score};

/// The reference shows at most this many nodes per kind, so one enormous group cannot bury
/// the others.
const MAX_RESULTS_PER_TYPE: usize = 3;

/// One searchable node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) id: NodeId,
    pub(crate) node_type: NodeType,
    pub(crate) label: String,
    pub(crate) snippet: String,
    haystack: String,
    /// What the *title* is scored against — the label, except for results, which match only on
    /// their rows.
    title_match: String,
}

/// One entry that survived a query, with where the query hit its label.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hit {
    pub(crate) entry: Entry,
    /// Character offsets in `label`, for highlighting. Empty when the hit came from elsewhere.
    pub(crate) label_match: Vec<usize>,
}

/// Hits of one kind, in score order. The kind whose best hit leads comes first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Group {
    pub(crate) node_type: NodeType,
    pub(crate) hits: Vec<Hit>,
}

/// Describes every node that has something to search, in page order.
pub(crate) fn entries<'a>(
    nodes: impl IntoIterator<Item = &'a Node>,
    rows: impl Fn(&NodeId) -> Option<&'a ResultSet>,
) -> Vec<Entry> {
    nodes
        .into_iter()
        .filter_map(|node| entry(node, rows(&node.id)))
        .collect()
}

/// Scores `entries`, groups the survivors by kind and caps each group.
///
/// An empty query is not a search: the overlay says what to type instead of listing the page.
pub(crate) fn search(entries: &[Entry], query: &str) -> Vec<Group> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(f64, Hit)> = entries
        .iter()
        .filter_map(|entry| rank(entry, query))
        .collect();
    // Descending, and stable, so equally good nodes keep their page order.
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut groups: Vec<Group> = Vec::new();
    for (_, hit) in scored {
        let node_type = hit.entry.node_type;
        if let Some(group) = groups.iter_mut().find(|group| group.node_type == node_type) {
            if group.hits.len() < MAX_RESULTS_PER_TYPE {
                group.hits.push(hit);
            }
        } else {
            groups.push(Group {
                node_type,
                hits: vec![hit],
            });
        }
    }
    groups
}

fn rank(entry: &Entry, query: &str) -> Option<(f64, Hit)> {
    let title = score(&entry.title_match, query).filter(|found| found.score >= MATCH_THRESHOLD);
    let body = score(&entry.haystack, query).filter(|found| found.score >= MATCH_THRESHOLD);
    let best = [title.as_ref(), body.as_ref()]
        .into_iter()
        .flatten()
        .map(|found| found.score)
        .fold(0.0_f64, f64::max);
    if best <= 0.0 {
        return None;
    }
    // Highlighting marks the line the row actually draws, so it is scored against the label
    // rather than inherited from whichever key ranked the node.
    let label_match = score(&entry.label, query)
        .filter(|found| found.score >= MATCH_THRESHOLD)
        .map_or_else(Vec::new, |found: Match| found.indices);
    Some((
        best,
        Hit {
            entry: entry.clone(),
            label_match,
        },
    ))
}

/// One searchable node, or `None` for a kind with nothing to say.
fn entry(node: &Node, rows: Option<&ResultSet>) -> Option<Entry> {
    let described = describe::describe(node, rows)?;

    // A result's title is its SQL, which the query node behind it already matches; leaving it
    // out is what stops one statement being listed twice under two kinds.
    let title_match = if described.node_type == NodeType::Result {
        String::new()
    } else {
        described.label.clone()
    };
    Some(Entry {
        id: node.id.clone(),
        node_type: described.node_type,
        label: described.label,
        snippet: described.snippet,
        haystack: described.haystack,
        title_match,
    })
}

#[cfg(test)]
mod tests {
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{Cell, Column, Node, NodeKind, NodeType, ResultData, ResultSet, TextData};

    use super::{entries, search};

    fn at(kind: NodeKind) -> Node {
        let mut node = Node::new(
            NodeType::Text,
            Rect::new(Point::new(0.0, 0.0), Size::new(300.0, 200.0)),
        );
        node.kind = kind;
        node
    }

    fn text(body: &str) -> Node {
        at(NodeKind::Text(TextData {
            text: body.to_string(),
        }))
    }

    fn result(query: &str) -> Node {
        at(NodeKind::Result(ResultData {
            query: query.to_string(),
            ..ResultData::default()
        }))
    }

    fn rows() -> ResultSet {
        ResultSet::new(
            vec![Column::new("customer", "TEXT")],
            vec![
                vec![Cell::Text("northwind".to_string())],
                vec![Cell::Text("umbrella".to_string())],
            ],
        )
    }

    fn labels(groups: &[super::Group]) -> Vec<String> {
        groups
            .iter()
            .flat_map(|group| group.hits.iter())
            .map(|hit| hit.entry.label.clone())
            .collect()
    }

    /// An empty query lists nothing: the panel says what to type rather than showing the page.
    #[test]
    fn an_empty_query_is_not_a_search() {
        let nodes = [text("grocery list")];
        let found = search(&entries(&nodes, |_| None), "  ");
        assert!(found.is_empty());
    }

    #[test]
    fn a_node_matches_on_the_text_it_holds() {
        let nodes = [text("grocery list"), text("deployment runbook")];
        let found = search(&entries(&nodes, |_| None), "runbook");
        assert_eq!(labels(&found), ["deployment runbook"]);
    }

    /// The load-bearing one: a result is named by its SQL but must not *match* on it, or one
    /// statement would be listed twice — once as the query node, once as its result.
    #[test]
    fn a_result_matches_on_its_cells_and_never_on_its_query() {
        let node = result("select * from invoices");
        let rows = rows();
        let found = search(
            &entries(std::slice::from_ref(&node), |_| Some(&rows)),
            "umbrella",
        );
        assert_eq!(found.len(), 1, "the cell matched");

        let found = search(
            &entries(std::slice::from_ref(&node), |_| Some(&rows)),
            "invoices",
        );
        assert!(found.is_empty(), "the query behind it did not");
    }

    /// One enormous group must not bury the others.
    #[test]
    fn a_group_is_capped_so_every_kind_stays_reachable() {
        let nodes: Vec<_> = (0..6)
            .map(|index| text(&format!("report {index}")))
            .collect();
        let found = search(&entries(&nodes, |_| None), "report");
        assert_eq!(found.len(), 1, "one group");
        assert_eq!(found[0].hits.len(), super::MAX_RESULTS_PER_TYPE);
    }
}
