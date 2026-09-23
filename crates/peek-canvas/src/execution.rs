//! Where a query's output lands on the canvas, and what feeds into the query first.
//!
//! Ported from `~/labs/peek/src/canvas/executeQueries.ts`. Everything here is pure: the actual
//! round trip to the database belongs to `peek-db`, and the view calls [`Document::place_result`]
//! or [`Document::place_query_error`] with what comes back.

use std::collections::BTreeMap;

use peek_document::geometry::{Point, Rect, Size};
use peek_document::{
    EdgeId, NodeData, NodeId, NodeType, ResultSet, VariableData, VariableRow, VariableValue,
};
use serde_json::{Map, Value};

use crate::history::EditKind;
use crate::model::Document;

/// Gap between a query node and the result it produces, and between stacked results.
const NODE_GAP: f64 = 50.0;
/// Fallbacks for a source node that has never been measured, from `executeQueries.ts`.
const DEFAULT_SOURCE_WIDTH: f64 = 350.0;
const DEFAULT_SOURCE_HEIGHT: f64 = 240.0;
const DEFAULT_RESULT_HEIGHT: f64 = 440.0;

const EMPTY_RESULT_SIZE: Size = Size {
    width: 400.0,
    height: 600.0,
};
const ERROR_NODE_SIZE: Size = Size {
    width: 400.0,
    height: 300.0,
};

const RESULT_ROW_HEIGHT: f64 = 50.0;
const RESULT_HEIGHT_PADDING: f64 = 140.0;
const MAX_RESULT_HEIGHT: f64 = 1500.0;

/// A chart node's shape, from `createChart.ts`: as wide as the result that fed it but never
/// narrower than this, always this tall, and sitting above the result with a gap.
const CHART_MIN_WIDTH: f64 = 500.0;
const CHART_HEIGHT: f64 = 500.0;
const CHART_GAP: f64 = 40.0;
/// How far to the right of a result a variable spawned from it sits (`useCellContextMenu.ts`).
const VARIABLE_GAP: f64 = 40.0;

/// How big a result node should be for the rows it holds.
///
/// Width is the sum of a per-type width over the columns, so a table of uuids opens wide and a
/// table of counts opens narrow; height grows with the row count and then stops, because past
/// about 27 rows the node is scrolling anyway.
#[must_use]
pub fn result_size(rows: &ResultSet) -> Size {
    if rows.is_empty() {
        return EMPTY_RESULT_SIZE;
    }
    let width: f64 = rows
        .columns()
        .iter()
        .map(|column| peek_document::placement_column_width(&column.sql_type))
        .sum();
    #[allow(
        clippy::cast_precision_loss,
        reason = "a row count far below 2^53 converts exactly"
    )]
    let height = (rows.row_count() as f64).mul_add(RESULT_ROW_HEIGHT, RESULT_HEIGHT_PADDING);
    Size::new(
        width.max(peek_document::MIN_RESULT_WIDTH),
        height.min(MAX_RESULT_HEIGHT),
    )
}

/// A result's rows as the chart node reads them: one map per row, column name to value.
///
/// `buildChartData` in `createChart.ts`. Two rules carry the whole thing: a NULL contributes no
/// key at all, so a gap in a column is a gap in the series rather than a zero, and a numeric
/// column that arrived as text — NUMERIC and friends ride as strings to keep their precision —
/// is coerced, so it plots as a series instead of being read as a row of labels.
#[must_use]
pub fn chart_series(rows: &ResultSet) -> Vec<Map<String, Value>> {
    (0..rows.row_count())
        .map(|row| {
            rows.columns()
                .iter()
                .enumerate()
                .filter_map(|(index, column)| {
                    let value = chart_value(rows.cell(row, index)?, &column.sql_type)?;
                    Some((column.name.clone(), value))
                })
                .collect()
        })
        .collect()
}

fn chart_value(cell: &peek_document::Cell, sql_type: &str) -> Option<Value> {
    use peek_document::Cell;
    match cell {
        Cell::Null | Cell::Undecodable | Cell::Json(_) | Cell::Bool(_) => None,
        Cell::Int(number) => Some(Value::from(*number)),
        Cell::Float(number) => serde_json::Number::from_f64(*number).map(Value::Number),
        Cell::Text(text) if peek_document::is_numeric(sql_type) => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        Cell::Text(text) => Some(Value::String(text.clone())),
    }
}

impl Document {
    /// Every `@name` visible to `target`, merged from the variable nodes feeding it.
    ///
    /// Incoming edges are walked in **edge-id order** so that two variable nodes defining the
    /// same name resolve the same way on every run — the reference sorts by `edge.id` for
    /// exactly this reason, and later edges win.
    #[must_use]
    pub fn variables_for(&self, target: &NodeId) -> BTreeMap<String, String> {
        let mut sources: Vec<&NodeId> = self
            .edges()
            .iter()
            .filter(|edge| &edge.target == target)
            .map(|edge| &edge.source)
            .collect();
        sources.sort_by(|left, right| {
            EdgeId::between(left, target).cmp(&EdgeId::between(right, target))
        });

        let mut merged = BTreeMap::new();
        for source in sources {
            let Some(node) = self.node(source) else {
                continue;
            };
            let Some(data) = VariableData::get(&node.kind) else {
                continue;
            };
            for row in &data.rows {
                if row.name.is_empty() {
                    continue;
                }
                merged.insert(row.name.clone(), sql_fragment(&row.value));
            }
        }
        merged
    }

    /// The query node a result came from, found through the edge that placed it.
    ///
    /// Re-running that node is how a result refreshes after a row is edited or deleted: it
    /// re-resolves the variables, re-places the rows and clears any error, all through the one
    /// path — the reference does the same rather than re-issuing the SQL by hand.
    #[must_use]
    pub fn source_query_of(&self, result: &NodeId) -> Option<NodeId> {
        self.edges()
            .iter()
            .filter(|edge| &edge.target == result)
            .map(|edge| edge.source.clone())
            .find(|source| {
                self.node(source)
                    .is_some_and(|node| node.node_type() == Some(NodeType::Query))
            })
    }

    /// Places or refreshes the result of running `query` on `source`, and stores its rows.
    ///
    /// `index` distinguishes the results of a multi-statement run; the node id is derived from
    /// the query's, so re-running updates the same node rather than littering the canvas.
    /// `previous` is the result placed just before this one in the same run, which this one
    /// stacks under.
    ///
    /// Returns the result node's id, and whether it was newly created — only new nodes are
    /// selected and flown to.
    pub fn place_result(
        &mut self,
        source: &NodeId,
        query: (&str, usize),
        rows: ResultSet,
    ) -> (NodeId, bool) {
        let (query, index) = query;
        let id = NodeId::result_of(source, index);
        let size = result_size(&rows);
        let existed = self.node(&id).is_some();
        let placed = id.clone();
        // `useChartSync`: an empty run is skipped rather than wiping the chart's saved series.
        let series = (!rows.is_empty()).then(|| chart_series(&rows));
        self.transaction_of(EditKind::Structure, |document| {
            document.place_result_inner(source, (query, index), (placed.clone(), size, existed));
            if let Some(series) = series {
                document.replot_charts_of(&placed, &series);
            }
        });
        self.set_result(id.clone(), rows);
        (id, !existed)
    }

    /// Pushes a result's fresh series into every chart it is wired to, keeping each chart's type.
    fn replot_charts_of(&mut self, result: &NodeId, series: &[Map<String, Value>]) {
        let charts: Vec<NodeId> = self
            .edges()
            .iter()
            .filter(|edge| &edge.source == result)
            .map(|edge| edge.target.clone())
            .filter(|target| {
                self.node(target)
                    .is_some_and(|node| node.node_type() == Some(NodeType::Barchart))
            })
            .collect();
        for chart in charts {
            self.update_data::<peek_document::BarChartData>(&chart, |data| {
                data.data = series.to_vec();
            });
        }
    }

    fn place_result_inner(
        &mut self,
        source: &NodeId,
        query: (&str, usize),
        placed: (NodeId, Size, bool),
    ) {
        let (query, index) = query;
        let (id, size, existed) = placed;
        if existed {
            // A live query re-runs every ten seconds; resizing then would fight the user every
            // time they dragged the node's corner.
            if !self.is_live_query(source) {
                self.set_size(&id, size);
            }
        } else {
            let origin = self.next_result_origin(source, self.result_stack_anchor(source, index));
            self.insert_node(id.clone(), NodeType::Result, Rect::new(origin, size));
            self.connect(source, &id);
        }

        self.update_data::<peek_document::ResultData>(&id, |data| {
            data.query = query.to_string();
        });
        // Success clears the error the previous run left behind.
        self.remove_nodes(&[NodeId::error_of(source)]);
    }

    /// Places or refreshes the chart of a result's rows.
    ///
    /// `createChart.ts`. The id is derived from the result's, so charting the same result twice
    /// re-plots the node already on the canvas rather than stacking another one on top of it —
    /// the rule `place_result` follows for the same reason.
    ///
    /// Returns the chart's id and whether it was newly created, or `None` when the result has no
    /// rows to plot. Only a new node is worth flying the camera to.
    pub fn place_chart(&mut self, result: &NodeId) -> Option<(NodeId, bool)> {
        let rows = self.result(result)?;
        if rows.is_empty() {
            return None;
        }
        let series = chart_series(rows);
        let id = NodeId::chart_of(result);
        let existed = self.node(&id).is_some();
        let placed = id.clone();
        // Insert, connect and plot are one edit: undoing a chart should not leave an empty one.
        self.transaction_of(EditKind::Structure, |document| {
            document.place_chart_inner(result, (placed, existed), series);
        });
        Some((id, !existed))
    }

    fn place_chart_inner(
        &mut self,
        result: &NodeId,
        placed: (NodeId, bool),
        series: Vec<Map<String, Value>>,
    ) {
        let (id, existed) = placed;
        if !existed {
            let source = self.node(result);
            let anchor = source.map_or_else(Point::default, |node| node.position);
            let width = source
                .and_then(|node| node.width)
                .unwrap_or(DEFAULT_SOURCE_WIDTH)
                .max(CHART_MIN_WIDTH);
            // Above the result rather than beside it: a result is already flanked by its query on
            // one side and whatever it was followed into on the other.
            let origin = Point::new(anchor.x, anchor.y - CHART_HEIGHT - CHART_GAP);
            let size = Size::new(width, CHART_HEIGHT);
            self.insert_node(id.clone(), NodeType::Barchart, Rect::new(origin, size));
            self.connect(result, &id);
        }
        self.update_data::<peek_document::BarChartData>(&id, |data| {
            data.data = series;
            // Only on creation: re-charting must not undo a switch to lines or an area.
            if !existed {
                data.chart_type = Some(peek_document::ChartType::Bar);
            }
        });
    }

    /// Spawns a variable node holding a value taken out of a result, wired to it.
    ///
    /// `createVariableFromCell` / `useColumnAsVariable`. Unlike [`Document::place_chart`] the id
    /// is **not** derived from the result's: the reference mints a fresh node every time, so
    /// charting twice re-plots one chart but using two cells as variables leaves two nodes.
    ///
    /// Insert, connect and fill are one edit, so undo never leaves an empty variable behind.
    pub fn place_variable(&mut self, result: &NodeId, row: VariableRow) -> NodeId {
        let source = self.node(result);
        let anchor = source.map_or_else(Point::default, |node| node.position);
        let width = source
            .and_then(|node| node.width)
            .unwrap_or(DEFAULT_SOURCE_WIDTH);
        let origin = Point::new(anchor.x + width + VARIABLE_GAP, anchor.y);
        let bounds = Rect::new(origin, NodeType::Variable.default_size());

        self.transaction_of(EditKind::Structure, |document| {
            let id = document.create_node(NodeType::Variable, bounds);
            document.update_data::<VariableData>(&id, |data| {
                data.rows = vec![row];
            });
            document.connect(result, &id);
            id
        })
    }

    /// Places or refreshes the `query-error` node for a failed run.
    ///
    /// The edge runs **error → query**, the opposite direction from a result: the error is
    /// commentary on the query rather than something the query feeds.
    ///
    /// Returns the node's id and whether it was newly created, like [`Document::place_result`]:
    /// a run frames what it created, and refreshing the error already on screen created nothing.
    pub fn place_query_error(
        &mut self,
        source: &NodeId,
        query: &str,
        message: &str,
    ) -> (NodeId, bool) {
        let id = NodeId::error_of(source);
        let existed = self.node(&id).is_some();
        let placed = id.clone();
        self.transaction_of(EditKind::Structure, |document| {
            document.place_query_error_inner(source, query, (placed, message));
        });
        (id, !existed)
    }

    /// `focusCreated` in `executeQueries.ts`: a finished run selects the nodes it created and
    /// asks the view to frame exactly them.
    ///
    /// Nothing created means nothing moves — a re-run that refreshed the result already on
    /// screen leaves both the selection and the camera where the user left them.
    pub fn focus_created(&mut self, created: Vec<NodeId>) {
        if created.is_empty() {
            return;
        }
        self.request_framing(created.clone());
        self.select_only(created);
    }

    fn place_query_error_inner(&mut self, source: &NodeId, query: &str, placed: (NodeId, &str)) {
        let (id, message) = placed;
        if self.node(&id).is_none() {
            let anchor = self
                .node(source)
                .map_or_else(Point::default, |node| node.position);
            let height = self
                .node(source)
                .map_or(DEFAULT_SOURCE_HEIGHT, |node| node.size().height);
            let origin = Point::new(anchor.x, anchor.y + height + NODE_GAP);
            self.insert_node(
                id.clone(),
                NodeType::QueryError,
                Rect::new(origin, ERROR_NODE_SIZE),
            );
            self.connect(&id, source);
        }
        self.update_data::<peek_document::ErrorData>(&id, |data| {
            data.query_node_id = source.clone();
            data.query = query.to_string();
            data.message = message.to_string();
        });
    }

    /// The node a new result stacks under: the previous result of this run, if it is on the
    /// canvas, otherwise nothing and the result sits beside its query.
    fn result_stack_anchor(&self, source: &NodeId, index: usize) -> Option<Point> {
        let previous = NodeId::result_of(source, index.checked_sub(1)?);
        let node = self.node(&previous)?;
        let height = node.height.unwrap_or(DEFAULT_RESULT_HEIGHT);
        Some(Point::new(
            node.position.x,
            node.position.y + height + NODE_GAP,
        ))
    }

    fn next_result_origin(&self, source: &NodeId, stacked: Option<Point>) -> Point {
        if let Some(point) = stacked {
            return point;
        }
        let Some(node) = self.node(source) else {
            return Point::default();
        };
        let width = if node.width.is_some() || node.measured.is_some() {
            node.size().width
        } else {
            DEFAULT_SOURCE_WIDTH
        };
        Point::new(node.position.x + width + NODE_GAP, node.position.y)
    }

    fn is_live_query(&self, source: &NodeId) -> bool {
        self.node(source)
            .and_then(|node| peek_document::QueryData::get(&node.kind))
            .is_some_and(|data| data.live_interval_ms.is_some())
    }
}

/// A variable's value as it goes into SQL.
///
/// A list joins with `", "` after dropping blank lines: the list editor keeps one line per row,
/// so the trailing newline the user is bound to leave behind would otherwise become an empty
/// element and produce `1, 2, ` inside an `IN (…)`.
fn sql_fragment(value: &VariableValue) -> String {
    match value {
        VariableValue::One(text) => text.clone(),
        VariableValue::Many(lines) => lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    }
}

#[cfg(test)]
mod tests {
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{
        Cell, Column, LiveInterval, NodeData, NodeId, NodeType, QueryData, ResultSet, VariableData,
        VariableRow, VariableValue,
    };

    use super::{chart_series, result_size};
    use crate::Document;

    /// A result whose second column really holds numbers, so it can be charted.
    fn plottable(count: usize) -> ResultSet {
        ResultSet::new(
            vec![Column::new("name", "TEXT"), Column::new("total", "INT8")],
            (0..count)
                .map(|index| {
                    vec![
                        Cell::Text(format!("row {index}")),
                        Cell::Int(i64::try_from(index).unwrap()),
                    ]
                })
                .collect(),
        )
    }

    fn rows(columns: &[(&str, &str)], count: usize) -> ResultSet {
        let columns: Vec<Column> = columns
            .iter()
            .map(|(name, sql_type)| Column::new(*name, *sql_type))
            .collect();
        let width = columns.len();
        ResultSet::new(columns, vec![vec![Cell::Null; width]; count])
    }

    /// A document with one query node at a known place, so placement is checkable by arithmetic.
    fn with_query() -> (Document, NodeId) {
        let mut document = Document::load(peek_document::CanvasDocument::empty());
        let id = document.create_node(
            NodeType::Query,
            Rect::new(Point::new(100.0, 200.0), Size::new(420.0, 320.0)),
        );
        (document, id)
    }

    #[test]
    fn an_empty_result_gets_the_fixed_empty_size() {
        let size = result_size(&ResultSet::default());
        assert!((size.width - 400.0).abs() < f64::EPSILON);
        assert!((size.height - 600.0).abs() < f64::EPSILON);
    }

    #[test]
    fn width_is_the_sum_of_per_type_column_widths() {
        // uuid 440 + timestamp 280 + numeric 130 + other 250
        let set = rows(
            &[
                ("id", "UUID"),
                ("at", "TIMESTAMPTZ"),
                ("n", "INT4"),
                ("name", "VARCHAR"),
            ],
            1,
        );
        assert!((result_size(&set).width - 1100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_narrow_result_is_floored_and_a_tall_one_capped() {
        let narrow = rows(&[("n", "INT4")], 1);
        assert!((result_size(&narrow).width - 200.0).abs() < f64::EPSILON);

        let tall = rows(&[("n", "INT4")], 10_000);
        assert!((result_size(&tall).height - 1500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn height_grows_with_the_row_count() {
        // 3 * 50 + 140
        assert!((result_size(&rows(&[("n", "INT4")], 3)).height - 290.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_first_result_lands_to_the_right_of_its_query() {
        let (mut document, query) = with_query();
        let (id, created) =
            document.place_result(&query, ("select 1", 0), rows(&[("n", "INT4")], 1));
        assert!(created);
        let node = document.node(&id).expect("placed");
        // 100 + 420 + 50
        assert!((node.position.x - 570.0).abs() < f64::EPSILON);
        assert!((node.position.y - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn later_results_stack_under_the_previous_one() {
        let (mut document, query) = with_query();
        document.place_result(&query, ("a", 0), rows(&[("n", "INT4")], 1));
        let (second, _) = document.place_result(&query, ("b", 1), rows(&[("n", "INT4")], 1));
        let first = document.node(&NodeId::result_of(&query, 0)).unwrap();
        let expected_y = first.position.y + first.height.unwrap() + 50.0;
        let node = document.node(&second).unwrap();
        assert!((node.position.x - first.position.x).abs() < f64::EPSILON);
        assert!((node.position.y - expected_y).abs() < f64::EPSILON);
    }

    /// Re-running updates the node already on the canvas; the id is derived from the query's.
    #[test]
    fn re_running_updates_the_same_node_and_reports_it_is_not_new() {
        let (mut document, query) = with_query();
        let (first, created) = document.place_result(&query, ("a", 0), rows(&[("n", "INT4")], 1));
        assert!(created);
        let before = document.nodes().len();
        let (again, created_again) =
            document.place_result(&query, ("b", 0), rows(&[("n", "INT4")], 9));
        assert_eq!(first, again);
        assert!(!created_again, "an existing node is not flown to");
        assert_eq!(document.nodes().len(), before, "no second node appeared");
    }

    /// Polling every ten seconds must not undo a manual resize.
    #[test]
    fn a_live_querys_result_keeps_its_size_on_re_run() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), rows(&[("n", "INT4")], 1));
        document.set_size(&result, Size::new(900.0, 700.0));
        document.update_data::<QueryData>(&query, |data| {
            data.live_interval_ms = Some(LiveInterval::EveryMs(10_000));
        });

        document.place_result(&query, ("a", 0), rows(&[("n", "INT4")], 40));
        let node = document.node(&result).unwrap();
        assert!((node.width.unwrap() - 900.0).abs() < f64::EPSILON);
        assert!((node.height.unwrap() - 700.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_result_is_connected_from_its_query() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), rows(&[("n", "INT4")], 1));
        assert!(
            document
                .edges()
                .iter()
                .any(|edge| edge.source == query && edge.target == result)
        );
    }

    /// The error edge runs the other way: it is commentary on the query, not something the
    /// query feeds.
    #[test]
    fn an_error_node_is_connected_backwards_into_its_query() {
        let (mut document, query) = with_query();
        let (error, created) = document.place_query_error(&query, "select boom", "syntax error");
        assert!(created, "the first failure creates the node");
        assert!(
            document
                .edges()
                .iter()
                .any(|edge| edge.source == error && edge.target == query)
        );
        let node = document.node(&error).unwrap();
        // 200 + 320 + 50
        assert!((node.position.y - 570.0).abs() < f64::EPSILON);
        assert!((node.position.x - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_second_failure_refreshes_the_error_rather_than_adding_one() {
        let (mut document, query) = with_query();
        document.place_query_error(&query, "q", "first");
        let count = document.nodes().len();
        let (error, created) = document.place_query_error(&query, "q", "second");
        assert_eq!(document.nodes().len(), count);
        assert!(!created, "refreshing it creates nothing to fly to");
        let data = peek_document::ErrorData::get(&document.node(&error).unwrap().kind).unwrap();
        assert_eq!(data.message, "second");
    }

    #[test]
    fn a_successful_run_clears_the_error_from_the_last_one() {
        let (mut document, query) = with_query();
        let (error, _) = document.place_query_error(&query, "q", "boom");
        document.place_result(&query, ("q", 0), rows(&[("n", "INT4")], 1));
        assert!(document.node(&error).is_none());
    }

    /// `focusCreated`: the nodes a run created are selected and handed to the camera. A re-run
    /// that only refreshed what was already there creates nothing, and so moves nothing.
    #[test]
    fn a_finished_run_selects_and_frames_only_what_it_created() {
        let (mut document, query) = with_query();
        let (result, created) = document.place_result(&query, ("q", 0), rows(&[("n", "INT4")], 1));
        assert!(created);

        document.focus_created(vec![result.clone()]);
        assert_eq!(
            document.selected().iter().collect::<Vec<_>>(),
            vec![&result]
        );
        assert_eq!(document.take_framing(), vec![result]);
        assert!(
            document.take_framing().is_empty(),
            "the request is drained, so the camera does not fly twice"
        );

        document.focus_created(Vec::new());
        assert!(document.take_framing().is_empty());
    }

    /// Placing a result is one undo step, not four (insert, connect, data, clear-error).
    #[test]
    fn placing_a_result_is_a_single_undo_step() {
        let (mut document, query) = with_query();
        document.checkpoint();
        let (result, _) = document.place_result(&query, ("a", 0), rows(&[("n", "INT4")], 1));
        document.checkpoint();

        assert!(document.undo(), "the placement undoes");
        assert!(
            document.node(&result).is_none(),
            "one undo removed the whole placement"
        );
        assert!(document.node(&query).is_some(), "the query itself survived");
    }

    #[test]
    fn variables_come_from_connected_variable_nodes() {
        let (mut document, query) = with_query();
        let variable = document.create_node(
            NodeType::Variable,
            Rect::new(Point::default(), Size::new(220.0, 120.0)),
        );
        document.update_data::<VariableData>(&variable, |data| {
            data.rows = vec![
                VariableRow {
                    name: "limit".to_string(),
                    value: VariableValue::One("10".to_string()),
                },
                VariableRow {
                    name: "ids".to_string(),
                    // The list editor leaves the trailing blank line behind.
                    value: VariableValue::Many(vec![
                        "1".to_string(),
                        "2".to_string(),
                        String::new(),
                    ]),
                },
            ];
        });
        document.connect(&variable, &query);

        let variables = document.variables_for(&query);
        assert_eq!(variables.get("limit").map(String::as_str), Some("10"));
        assert_eq!(
            variables.get("ids").map(String::as_str),
            Some("1, 2"),
            "blank lines are dropped so an IN list has no trailing comma"
        );
    }

    #[test]
    fn a_query_with_no_variable_sources_sees_nothing() {
        let (document, query) = with_query();
        assert!(document.variables_for(&query).is_empty());
    }

    /// Charting a result puts a chart above it, wired to it, holding its rows.
    #[test]
    fn charting_a_result_places_a_chart_above_it() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), plottable(3));
        let position = document.node(&result).unwrap().position;

        let (chart, created) = document.place_chart(&result).expect("rows to plot");

        assert!(created);
        let node = document.node(&chart).unwrap();
        assert_eq!(node.node_type(), Some(NodeType::Barchart));
        assert!(
            node.position.y < position.y,
            "above the result, not beside it: {:?} against {position:?}",
            node.position
        );
        assert!(
            document
                .edges()
                .iter()
                .any(|edge| edge.source == result && edge.target == chart)
        );
        let data = peek_document::BarChartData::get(&document.node(&chart).unwrap().kind).unwrap();
        assert_eq!(data.data.len(), 3, "one entry per row");
    }

    /// The chart's id comes from the result's, so re-charting re-plots rather than stacking a
    /// second node on the first.
    #[test]
    fn charting_twice_updates_the_same_node() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), plottable(3));
        let (first, _) = document.place_chart(&result).unwrap();

        document.place_result(&query, ("a", 0), plottable(5));
        let (second, created) = document.place_chart(&result).unwrap();

        assert_eq!(first, second);
        assert!(!created, "the node was already there");
        let data = peek_document::BarChartData::get(&document.node(&second).unwrap().kind).unwrap();
        assert_eq!(data.data.len(), 5, "and it re-plotted the new rows");
    }

    /// `useChartSync`: a chart follows its result's rows without being re-charted, and an
    /// empty run leaves the last series on screen.
    #[test]
    fn re_running_the_query_re_plots_its_chart() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), plottable(3));
        let (chart, _) = document.place_chart(&result).unwrap();
        let plotted = |document: &Document| {
            peek_document::BarChartData::get(&document.node(&chart).unwrap().kind)
                .unwrap()
                .data
                .len()
        };

        document.place_result(&query, ("a", 0), plottable(5));
        assert_eq!(plotted(&document), 5);

        document.place_result(&query, ("a", 0), ResultSet::default());
        assert_eq!(plotted(&document), 5);
    }

    /// A re-chart must not undo a switch to lines: the type is the user's, the data is the
    /// query's.
    #[test]
    fn re_charting_keeps_the_chart_type() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), plottable(2));
        let (chart, _) = document.place_chart(&result).unwrap();
        document.update_data::<peek_document::BarChartData>(&chart, |data| {
            data.chart_type = Some(peek_document::ChartType::Line);
        });

        document.place_chart(&result);

        let data = peek_document::BarChartData::get(&document.node(&chart).unwrap().kind).unwrap();
        assert_eq!(data.chart_type, Some(peek_document::ChartType::Line));
    }

    /// Placing the node, wiring it and plotting it is one edit: undo should not leave an empty
    /// chart behind.
    #[test]
    fn charting_is_a_single_undo_step() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), plottable(3));
        let before = document.nodes().len();

        let (chart, _) = document.place_chart(&result).unwrap();
        assert!(document.node(&chart).is_some());

        document.undo();
        assert_eq!(document.nodes().len(), before);
        assert!(document.node(&chart).is_none());
    }

    /// A variable spawned from a result sits beside it, wired to it, holding the value it was
    /// made from.
    #[test]
    fn a_variable_is_placed_beside_the_result_that_made_it() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), plottable(3));
        let node = document.node(&result).unwrap();
        let (position, width) = (node.position, node.width.unwrap());

        let variable = document.place_variable(
            &result,
            VariableRow {
                name: "name".to_string(),
                value: VariableValue::One("Lola".to_string()),
            },
        );

        let placed = document.node(&variable).unwrap();
        assert_eq!(placed.node_type(), Some(NodeType::Variable));
        assert!(
            (placed.position.x - (position.x + width + 40.0)).abs() < f64::EPSILON,
            "to the right of the result with a gap"
        );
        assert!(
            document
                .edges()
                .iter()
                .any(|edge| edge.source == result && edge.target == variable)
        );
        let data = VariableData::get(&placed.kind).unwrap();
        assert_eq!(data.rows.len(), 1);
        assert_eq!(data.rows[0].name, "name");
        assert_eq!(
            data.rows[0].value,
            VariableValue::One("Lola".to_string()),
            "a single cell keeps its raw value, unquoted"
        );
    }

    /// The id is minted, not derived: two cells used as variables are two nodes, where charting
    /// twice re-plots one chart.
    #[test]
    fn using_two_cells_as_variables_makes_two_nodes() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), plottable(3));
        let row = |name: &str| VariableRow {
            name: name.to_string(),
            value: VariableValue::One("x".to_string()),
        };

        let first = document.place_variable(&result, row("a"));
        let second = document.place_variable(&result, row("b"));

        assert_ne!(first, second);
        assert!(document.node(&first).is_some() && document.node(&second).is_some());
    }

    /// Node, edge and value are one edit: undo must not leave an empty variable behind.
    #[test]
    fn placing_a_variable_is_a_single_undo_step() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), plottable(2));
        let before = document.nodes().len();

        let variable = document.place_variable(
            &result,
            VariableRow {
                name: "ids".to_string(),
                value: VariableValue::Many(vec!["1".to_string(), "2".to_string()]),
            },
        );
        assert!(document.node(&variable).is_some());

        document.undo();
        assert_eq!(document.nodes().len(), before);
        assert!(document.node(&variable).is_none());
    }

    #[test]
    fn a_result_with_no_rows_has_nothing_to_chart() {
        let (mut document, query) = with_query();
        let (result, _) = document.place_result(&query, ("a", 0), ResultSet::default());
        assert!(document.place_chart(&result).is_none());
    }

    /// `buildChartData`'s two load-bearing rules: a NULL leaves the key out entirely, and a
    /// numeric column that arrived as text is coerced so it plots as a series.
    #[test]
    fn chart_series_skips_nulls_and_coerces_numeric_text() {
        let set = ResultSet::new(
            vec![
                Column::new("label", "TEXT"),
                Column::new("amount", "NUMERIC"),
                Column::new("missing", "INT4"),
            ],
            vec![vec![
                Cell::Text("a".to_string()),
                Cell::Text("12.5".to_string()),
                Cell::Null,
            ]],
        );

        let series = chart_series(&set);
        let row = &series[0];
        assert_eq!(
            row.get("label").and_then(serde_json::Value::as_str),
            Some("a")
        );
        assert_eq!(
            row.get("amount").and_then(serde_json::Value::as_f64),
            Some(12.5),
            "NUMERIC rides as text to keep its precision, and plots as a number"
        );
        assert!(!row.contains_key("missing"), "a NULL contributes no key");
    }
}
