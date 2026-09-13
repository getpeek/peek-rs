//! How wide each column of a result is.
//!
//! Ported from `~/labs/peek/src/canvas/nodes/Result/hooks/useColumnWidths.ts`. Widths are in
//! **world units**, the same space `ResultData::column_widths` is persisted in, and the view
//! multiplies by the camera's zoom on its way to pixels.

use std::collections::BTreeMap;

use peek_document::ResultSet;

/// Width of one character in the monospace face the table uses, and the padding around a cell.
/// Measuring text properly would mean a text-system round trip per column per frame; the
/// reference estimates, and a column that guesses a little wide is harmless next to that cost.
const MONO_CHAR: f64 = 7.2;
const CELL_PADDING: f64 = 28.0;

const MIN_COLUMN: f64 = 80.0;
/// A default only. An explicit width from the document is honoured however wide it is, so a
/// column the user dragged out stays dragged out.
const MAX_DEFAULT_COLUMN: f64 = 360.0;
/// Only the first rows are measured: a 6,515-row result would otherwise walk every cell to
/// decide a column width nobody is going to look at past the first screen.
const SAMPLE_ROWS: usize = 30;

/// The width every column gets, in world units, and their total.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ColumnWidths {
    widths: Vec<f64>,
    total: f64,
}

impl ColumnWidths {
    /// `available` is the body's own width in world units; when the columns do not fill it they
    /// are scaled up so the table always reaches the edge of the node rather than leaving a
    /// stripe of background down the right.
    pub(super) fn resolve(
        rows: &ResultSet,
        explicit: Option<&BTreeMap<String, f64>>,
        available: f64,
    ) -> Self {
        let natural: Vec<f64> = rows
            .columns()
            .iter()
            .enumerate()
            .map(|(index, column)| {
                explicit
                    .and_then(|widths| widths.get(&column.name))
                    .copied()
                    .unwrap_or_else(|| natural_width(rows, index, &column.name))
            })
            .collect();

        let total: f64 = natural.iter().sum();
        if total <= 0.0 {
            return Self {
                widths: natural,
                total: 0.0,
            };
        }
        if available <= total {
            return Self {
                widths: natural,
                total,
            };
        }
        let scale = available / total;
        Self {
            widths: natural.iter().map(|width| width * scale).collect(),
            total: available,
        }
    }

    pub(super) fn get(&self, index: usize) -> f64 {
        self.widths.get(index).copied().unwrap_or(MIN_COLUMN)
    }

    #[cfg(test)]
    pub(super) fn total(&self) -> f64 {
        self.total
    }
}

/// The width a column would like: the longest of its header and its sampled values.
fn natural_width(rows: &ResultSet, index: usize, header: &str) -> f64 {
    let longest = rows
        .rows()
        .iter()
        .take(SAMPLE_ROWS)
        .filter_map(|row| row.get(index))
        .map(|cell| cell.to_display_string().chars().count())
        .chain(std::iter::once(header.chars().count()))
        .max()
        .unwrap_or(0);

    #[allow(
        clippy::cast_precision_loss,
        reason = "a cell length far below 2^53 converts exactly"
    )]
    let natural = (longest as f64).mul_add(MONO_CHAR, CELL_PADDING).round();
    natural.clamp(MIN_COLUMN, MAX_DEFAULT_COLUMN)
}

#[cfg(test)]
mod tests {
    use peek_document::{Cell, Column, ResultSet};

    use super::{ColumnWidths, MAX_DEFAULT_COLUMN, MIN_COLUMN};

    fn rows(columns: &[(&str, &str)], cells: Vec<Vec<Cell>>) -> ResultSet {
        ResultSet::new(
            columns
                .iter()
                .map(|(name, sql_type)| Column::new(*name, *sql_type))
                .collect(),
            cells,
        )
    }

    #[test]
    fn a_short_column_gets_the_minimum() {
        let set = rows(&[("n", "INT4")], vec![vec![Cell::Int(1)]]);
        let widths = ColumnWidths::resolve(&set, None, 0.0);
        assert!((widths.get(0) - MIN_COLUMN).abs() < f64::EPSILON);
    }

    /// A single token or URL must not be allowed to open a column to the width of the screen.
    #[test]
    fn a_very_long_value_is_capped() {
        let set = rows(&[("t", "TEXT")], vec![vec![Cell::Text("x".repeat(500))]]);
        let widths = ColumnWidths::resolve(&set, None, 0.0);
        assert!((widths.get(0) - MAX_DEFAULT_COLUMN).abs() < f64::EPSILON);
    }

    /// The header counts too, or a column of nulls under a long name would be unreadable.
    #[test]
    fn the_header_sets_the_floor_for_an_empty_column() {
        let set = rows(
            &[("a_rather_long_column_name_here", "TEXT")],
            vec![vec![Cell::Null]],
        );
        let widths = ColumnWidths::resolve(&set, None, 0.0);
        assert!(widths.get(0) > MIN_COLUMN);
    }

    /// Past the sample the rows are not measured, so a wide value far down does not count.
    #[test]
    fn only_the_first_rows_are_sampled() {
        let mut cells = vec![vec![Cell::Int(1)]; 40];
        cells.push(vec![Cell::Text("x".repeat(200))]);
        let set = rows(&[("n", "INT4")], cells);
        let widths = ColumnWidths::resolve(&set, None, 0.0);
        assert!((widths.get(0) - MIN_COLUMN).abs() < f64::EPSILON);
    }

    /// A width the user dragged out is honoured however wide, unlike the default which is capped.
    #[test]
    fn an_explicit_width_overrides_and_is_not_capped() {
        let set = rows(&[("n", "INT4")], vec![vec![Cell::Int(1)]]);
        let explicit = [("n".to_string(), 900.0)].into_iter().collect();
        let widths = ColumnWidths::resolve(&set, Some(&explicit), 0.0);
        assert!((widths.get(0) - 900.0).abs() < f64::EPSILON);
    }

    /// Narrow columns stretch to fill the node, so the table never leaves a bare stripe.
    #[test]
    fn columns_expand_to_fill_the_body() {
        let set = rows(
            &[("a", "INT4"), ("b", "INT4")],
            vec![vec![Cell::Int(1), Cell::Int(2)]],
        );
        let widths = ColumnWidths::resolve(&set, None, 400.0);
        assert!((widths.total() - 400.0).abs() < f64::EPSILON);
        assert!((widths.get(0) + widths.get(1) - 400.0).abs() < 0.001);
    }

    /// Wider than the node, they keep their width and the table scrolls sideways instead.
    #[test]
    fn columns_wider_than_the_body_are_left_alone() {
        let set = rows(
            &[("a", "TEXT"), ("b", "TEXT")],
            vec![vec![Cell::Text("x".repeat(60)), Cell::Text("y".repeat(60))]],
        );
        let widths = ColumnWidths::resolve(&set, None, 100.0);
        assert!(widths.total() > 100.0, "no shrinking to fit");
    }

    #[test]
    fn a_result_with_no_columns_has_no_width() {
        let widths = ColumnWidths::resolve(&ResultSet::default(), None, 500.0);
        assert!((widths.total() - 0.0).abs() < f64::EPSILON);
    }
}
