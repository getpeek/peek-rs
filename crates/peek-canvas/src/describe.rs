//! What a node *says*, in words: the port of `~/labs/peek/src/page-search/searchCorpus.ts`'s
//! `describeNode`.
//!
//! One description, two consumers, exactly as in the reference: page search matches against it,
//! and the AI grouping prompt describes the page with it. Adding a node kind therefore means
//! teaching one match, not two.

use peek_document::{
    BarChartData, Cell, ChartType, Node, NodeKind, NodeType, QueryData, ResultData, ResultSet,
    TableDefinitionData, VariableData, VariableValue,
};

/// Cells past this row count are never described. Enough to find a node by a value it shows
/// without stringifying a 100k-row result — `MAX_SEARCHED_ROWS`.
const MAX_SEARCHED_ROWS: usize = 100;

/// A label is cut here, which is `title` in the reference.
const LABEL_LIMIT: usize = 60;

/// How much of a query a node's heading shows — `nodeHeading` in `QueryNode.tsx`.
const HEADING_LIMIT: usize = 60;

/// What one node says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Described {
    pub node_type: NodeType,
    /// The node's first line: its title.
    pub label: String,
    /// Its second line: the one detail that tells two nodes of a kind apart.
    pub snippet: String,
    /// Everything else it holds — a query's SQL, an agent's transcript, a result's cells.
    pub haystack: String,
}

/// Describes `node`, with `rows` when it is a result whose sidecar has landed.
///
/// `None` for the kinds with nothing to say: freehand strokes, insert forms, and the Activity
/// node, whose rows are a live poll rather than persisted text.
#[must_use]
pub fn describe(node: &Node, rows: Option<&ResultSet>) -> Option<Described> {
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
    Some(Described {
        node_type,
        label,
        snippet,
        haystack,
    })
}

/// A statement reduced to one line, for the nodes titled by the SQL behind them.
///
/// Every line is joined rather than only the first taken: a query formatted across lines starts
/// with a bare `SELECT`, and a node titled "SELECT" says nothing about which one it is.
///
/// One divergence: `nodeHeading` appends `...` unconditionally, so a one-word query reads
/// `SELECT 1...`. The ellipsis is only appended here when something was actually cut.
#[must_use]
pub fn heading(query: &str) -> String {
    let joined = query
        .trim_start()
        .trim_start_matches("--")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        return "result".to_string();
    }
    if joined.chars().count() <= HEADING_LIMIT {
        return joined;
    }
    let cut: String = joined.chars().take(HEADING_LIMIT).collect();
    format!("{cut}...")
}

/// What one node contributes: the first line, the second, and everything else it says.
type Parts = (String, String, String);

fn query_parts(data: &QueryData) -> Parts {
    let description = data.description.clone().unwrap_or_default();
    let label = if description.is_empty() {
        heading(&data.query)
    } else {
        description.clone()
    };
    (
        label,
        collapse(&data.query),
        format!("{description} {}", data.query),
    )
}

fn result_parts(data: &ResultData, rows: Option<&ResultSet>) -> Parts {
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
        columns.join(" \u{b7} ")
    };
    (heading(&data.query), snippet, cells(rows))
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

fn variable_parts(data: &VariableData) -> Parts {
    let pairs: Vec<String> = data
        .rows
        .iter()
        .map(|row| format!("{} = {}", row.name, value(&row.value)))
        .collect();
    let names: Vec<&str> = data.rows.iter().map(|row| row.name.as_str()).collect();
    (
        cut(&names.join(", ")),
        collapse(&pairs.join(" \u{b7} ")),
        pairs.join(" "),
    )
}

fn table_parts(data: &TableDefinitionData) -> Parts {
    let columns: Vec<String> = data
        .columns
        .iter()
        .map(|(name, sql_type)| format!("{name} {sql_type}"))
        .collect();
    let names: Vec<&str> = data.columns.iter().map(|(name, _)| name.as_str()).collect();
    (
        data.table.clone(),
        collapse(&names.join(" \u{b7} ")),
        std::iter::once(data.table.clone())
            .chain(columns)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn chart_parts(data: &BarChartData) -> Parts {
    let columns: Vec<&str> = data
        .data
        .first()
        .map(|row| row.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let kind = data.chart_type.map_or("bar", chart_name);
    let label = if columns.is_empty() {
        "Chart".to_string()
    } else {
        cut(&columns.join(" \u{b7} "))
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
        .flat_map(|row| row.iter().map(Cell::to_display_string))
        .collect::<Vec<_>>()
        .join(" ")
}

fn value(value: &VariableValue) -> String {
    match value {
        VariableValue::One(one) => one.clone(),
        VariableValue::Many(many) => many.join(", "),
    }
}

const fn chart_name(kind: ChartType) -> &'static str {
    match kind {
        ChartType::Bar => "bar",
        ChartType::Line => "line",
        ChartType::Area => "area",
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
    use super::{describe, heading};
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{Cell, Column, Node, NodeKind, NodeType, ResultData, ResultSet};

    fn result(query: &str) -> Node {
        let mut node = Node::new(
            NodeType::Result,
            Rect::new(Point::new(0.0, 0.0), Size::new(300.0, 200.0)),
        );
        node.kind = NodeKind::Result(ResultData {
            query: query.to_string(),
            ..ResultData::default()
        });
        node
    }

    /// The row's second line names the columns, which is what tells two results of the same
    /// query shape apart at a glance.
    #[test]
    fn a_results_snippet_is_its_columns() {
        let rows = ResultSet::new(
            vec![Column::new("customer", "TEXT")],
            vec![vec![Cell::Text("northwind".to_string())]],
        );
        let described = describe(&result("select * from invoices"), Some(&rows))
            .expect("a result is described");

        assert_eq!(described.label, "select * from invoices");
        assert_eq!(described.snippet, "customer");
        assert_eq!(described.haystack, "northwind");
    }

    /// Freehand strokes, insert forms and the Activity node hold nothing to describe.
    #[test]
    fn kinds_with_nothing_to_say_are_left_out() {
        let mut node = result("select 1");
        node.kind = NodeKind::Draw(peek_document::DrawData::default());

        assert!(describe(&node, None).is_none());
    }

    /// `nodeHeading` joins the whole statement onto one line. Taking only the first would title
    /// every formatted query `SELECT`, which is what the node header used to show.
    #[test]
    fn the_heading_joins_a_multi_line_query() {
        assert_eq!(
            heading("SELECT\n  DATE_TRUNC('month', s.created_at)\nFROM subscriptions s"),
            "SELECT DATE_TRUNC('month', s.created_at) FROM subscriptions ..."
        );
    }

    #[test]
    fn the_heading_is_the_query_on_one_line() {
        assert_eq!(
            heading("\n\n  select * from users  "),
            "select * from users"
        );
    }

    /// `nodeHeading` strips a leading comment marker, so a documented query is not titled `--`.
    #[test]
    fn a_leading_comment_marker_is_stripped() {
        assert_eq!(heading("-- everyone\nselect 1"), "everyone select 1");
    }

    #[test]
    fn a_long_query_is_cut_with_an_ellipsis() {
        let long = format!("select {}", "x".repeat(100));
        let heading = heading(&long);
        assert_eq!(
            heading.chars().count(),
            63,
            "60 characters plus the ellipsis"
        );
        assert!(heading.ends_with("..."));
    }

    #[test]
    fn an_empty_query_still_has_a_heading() {
        assert_eq!(heading(""), "result");
    }

    /// Multi-byte text must be cut on a character boundary, not a byte one.
    #[test]
    fn a_long_multibyte_query_does_not_panic() {
        let long = "é".repeat(200);
        assert_eq!(heading(&long).chars().count(), 63);
    }
}
