//! What page search matches against: `~/labs/peek/src/page-search/searchCorpus.ts` and
//! `useNodeSearch.ts`.
//!
//! One [`Entry`] per searchable node on the active page. `label` is the row's first line,
//! `snippet` its second, and `haystack` everything else the node says — a query's SQL, an
//! agent's transcript, a result's cells. A result node's *title* is its SQL, which is already
//! the query node's job to match, so results match on their data alone.

use peek_document::{Node, NodeId, NodeKind, NodeType, ResultSet, VariableValue};

use crate::fuzzy::{MATCH_THRESHOLD, Match, score};

/// Cells past this row count never enter the haystack. Enough to find a node by a value it
/// shows without stringifying a 100k-row result on every keystroke — `MAX_SEARCHED_ROWS`.
const MAX_SEARCHED_ROWS: usize = 100;

/// The reference shows at most this many nodes per kind, so one enormous group cannot bury
/// the others.
const MAX_RESULTS_PER_TYPE: usize = 3;

/// A label is cut here, which is `title` in the reference.
const LABEL_LIMIT: usize = 60;

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
        .filter_map(|node| describe(node, rows(&node.id)))
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

/// What one node contributes: the row's first line, its second, and everything else it says.
type Parts = (String, String, String);

/// `None` for kinds with nothing meaningful to search: freehand strokes, insert forms, and the
/// Activity node, whose rows are a live poll rather than persisted text.
fn describe(node: &Node, rows: Option<&ResultSet>) -> Option<Entry> {
    let node_type = node.node_type()?;
    let (label, snippet, haystack) = match &node.kind {
        NodeKind::Query(data) => query_parts(data),
        NodeKind::Result(data) => result_parts(data, rows),
        NodeKind::Agent(data) => agent_parts(data),
        NodeKind::Text(data) => (cut(&data.text), collapse(&data.text), data.text.clone()),
        NodeKind::Variable(data) => variable_parts(data),
        NodeKind::TableDefinition(data) => table_parts(data),
        NodeKind::QueryError(data) => (
            cut(&data.message),
            collapse(&data.query),
            format!("{} {}", data.message, data.query),
        ),
        NodeKind::Barchart(data) => chart_parts(data),
        NodeKind::ResultInsertForm(_)
        | NodeKind::Draw(_)
        | NodeKind::Activity(_)
        | NodeKind::Unknown => return None,
    };

    // A result's title is its SQL, which the query node behind it already matches; leaving it
    // out is what stops one statement being listed twice under two kinds.
    let title_match = if node_type == NodeType::Result {
        String::new()
    } else {
        label.clone()
    };
    Some(Entry {
        id: node.id.clone(),
        node_type,
        label,
        snippet,
        haystack,
        title_match,
    })
}

fn query_parts(data: &peek_document::QueryData) -> Parts {
    let description = data.description.clone().unwrap_or_default();
    let label = if description.is_empty() {
        crate::node::result::heading(&data.query)
    } else {
        description.clone()
    };
    (
        label,
        collapse(&data.query),
        format!("{description} {}", data.query),
    )
}

fn result_parts(data: &peek_document::ResultData, rows: Option<&ResultSet>) -> Parts {
    let columns: Vec<&str> = rows
        .map(|rows| {
            rows.columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect()
        })
        .unwrap_or_default();
    let snippet = if columns.is_empty() {
        collapse(&data.query)
    } else {
        columns.join(" · ")
    };
    (
        crate::node::result::heading(&data.query),
        snippet,
        cells(rows),
    )
}

fn agent_parts(data: &peek_document::AgentData) -> Parts {
    let chat: Vec<&str> = data
        .messages
        .iter()
        .filter(|message| message.kind == "user" || message.kind == "assistant")
        .map(|message| message.message.as_str())
        .collect();
    let last = chat.last().copied().unwrap_or(&data.query);
    (
        cut(&data.query),
        collapse(last),
        std::iter::once(data.query.as_str())
            .chain(chat)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn variable_parts(data: &peek_document::VariableData) -> Parts {
    let pairs: Vec<String> = data
        .rows
        .iter()
        .map(|row| format!("{} = {}", row.name, value(&row.value)))
        .collect();
    let names: Vec<&str> = data.rows.iter().map(|row| row.name.as_str()).collect();
    (
        cut(&names.join(", ")),
        collapse(&pairs.join(" · ")),
        pairs.join(" "),
    )
}

fn table_parts(data: &peek_document::TableDefinitionData) -> Parts {
    let columns: Vec<String> = data
        .columns
        .iter()
        .map(|(name, sql_type)| format!("{name} {sql_type}"))
        .collect();
    let names: Vec<&str> = data.columns.iter().map(|(name, _)| name.as_str()).collect();
    (
        data.table.clone(),
        collapse(&names.join(" · ")),
        std::iter::once(data.table.clone())
            .chain(columns)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn chart_parts(data: &peek_document::BarChartData) -> Parts {
    let columns: Vec<&str> = data
        .data
        .first()
        .map(|row| row.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let kind = data.chart_type.map_or("bar", chart_name);
    let label = if columns.is_empty() {
        "Chart".to_string()
    } else {
        cut(&columns.join(" · "))
    };
    (
        label,
        kind.to_string(),
        format!("{} {kind}", columns.join(" ")),
    )
}

fn cells(rows: Option<&ResultSet>) -> String {
    let Some(rows) = rows else {
        return String::new();
    };
    rows.rows()
        .iter()
        .take(MAX_SEARCHED_ROWS)
        .flat_map(|row| row.iter().map(peek_document::Cell::to_display_string))
        .collect::<Vec<_>>()
        .join(" ")
}

fn value(value: &VariableValue) -> String {
    match value {
        VariableValue::One(one) => one.clone(),
        VariableValue::Many(many) => many.join(", "),
    }
}

const fn chart_name(kind: peek_document::ChartType) -> &'static str {
    match kind {
        peek_document::ChartType::Bar => "bar",
        peek_document::ChartType::Line => "line",
        peek_document::ChartType::Area => "area",
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cut(text: &str) -> String {
    let collapsed = collapse(text);
    if collapsed.chars().count() <= LABEL_LIMIT {
        return collapsed;
    }
    collapsed.chars().take(LABEL_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{Cell, Column, Node, NodeKind, NodeType, ResultData, ResultSet, TextData};

    use super::{describe, entries, search};

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

    /// The row's second line names the columns, which is what tells two results of the same
    /// query shape apart at a glance.
    #[test]
    fn a_results_snippet_is_its_columns() {
        let node = result("select * from invoices");
        let rows = rows();
        let entry = describe(&node, Some(&rows)).expect("a result is searchable");
        assert_eq!(entry.snippet, "customer");
        assert_eq!(entry.label, "select * from invoices");
    }

    /// Freehand strokes, insert forms and the Activity node hold nothing to search.
    #[test]
    fn kinds_with_nothing_to_search_are_left_out() {
        let node = at(NodeKind::Draw(peek_document::DrawData::default()));
        assert!(describe(&node, None).is_none());
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
