//! Statistics over a selected rectangle of cells.
//!
//! Ported from `aggregate.ts`. The rule that matters: **one non-numeric cell and there is no
//! answer at all**. Summing what happens to parse inside a mixed selection would put a number
//! under a column of names, which is worse than saying nothing.

use peek_document::{Cell, ResultSet};

use super::selection::CellRect;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Aggregates {
    /// `f64` rather than `usize` because it is only ever formatted alongside the others, and a
    /// selection never approaches the point where a double stops counting exactly.
    pub(super) count: f64,
    pub(super) sum: f64,
    pub(super) average: f64,
    pub(super) minimum: f64,
    pub(super) maximum: f64,
}

/// A cell's value as a number, or `None` if it is not one.
///
/// `NUMERIC` and `DECIMAL` arrive as text to keep the precision a double would lose, so a string
/// counts — but only when its column is numeric. A `VARCHAR` holding "42" is not a number, or
/// selecting a column of postcodes would offer to average them.
fn numeric(cell: &Cell, sql_type: &str) -> Option<f64> {
    match cell {
        Cell::Int(value) =>
        {
            #[allow(
                clippy::cast_precision_loss,
                reason = "matching the reference, which works in JavaScript doubles throughout"
            )]
            Some(*value as f64)
        }
        Cell::Float(value) if value.is_finite() => Some(*value),
        Cell::Text(text) if peek_document::is_numeric(sql_type) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            trimmed
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
        }
        _ => None,
    }
}

/// `None` unless every cell in `rect` is numeric, and there is at least one.
///
/// `visible` maps display positions — which is what a rect holds — onto rows of the result.
pub(super) fn aggregate(rows: &ResultSet, rect: CellRect, visible: &[usize]) -> Option<Aggregates> {
    let mut values: Vec<f64> = Vec::new();
    for position in rect.rows() {
        let Some(row) = visible.get(position).copied() else {
            continue;
        };
        for column in rect.columns() {
            let Some(cell) = rows.cell(row, column) else {
                continue;
            };
            let sql_type = rows
                .columns()
                .get(column)
                .map_or("", |column| column.sql_type.as_str());
            // One non-numeric cell and the whole selection has no answer.
            values.push(numeric(cell, sql_type)?);
        }
    }
    if values.is_empty() {
        return None;
    }

    let sum: f64 = values.iter().sum();
    #[allow(
        clippy::cast_precision_loss,
        reason = "selection sizes are far below 2^53"
    )]
    let count = values.len() as f64;
    let average = sum / count;
    Some(Aggregates {
        count,
        sum,
        average,
        minimum: values.iter().copied().fold(f64::INFINITY, f64::min),
        maximum: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    })
}

/// A statistic as the toolbar shows it: grouped thousands, at most four decimals, and no
/// trailing zeroes — `Intl.NumberFormat(undefined, { maximumFractionDigits: 4 })`.
pub(super) fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    let rounded = (value * 10_000.0).round() / 10_000.0;
    let mut text = format!("{rounded:.4}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest.to_string()),
        None => ("", text),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole.to_string(), format!(".{fraction}")),
        None => (digits, String::new()),
    };
    format!("{sign}{}{fraction}", group_thousands(&whole))
}

fn group_thousands(whole: &str) -> String {
    let mut grouped = String::with_capacity(whole.len() + whole.len() / 3);
    for (offset, digit) in whole.chars().enumerate() {
        if offset > 0 && (whole.len() - offset).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use peek_document::{Cell, Column, ResultSet};

    use super::super::selection::CellRect;
    use super::{aggregate, format_number};

    fn rect(top: usize, bottom: usize, left: usize, right: usize) -> CellRect {
        CellRect {
            top,
            bottom,
            left,
            right,
        }
    }

    fn numbers() -> ResultSet {
        ResultSet::new(
            vec![Column::new("n", "INT4"), Column::new("amount", "NUMERIC")],
            vec![
                vec![Cell::Int(1), Cell::Text("10.5".into())],
                vec![Cell::Int(3), Cell::Text("20.5".into())],
            ],
        )
    }

    #[test]
    fn aggregates_a_numeric_rectangle() {
        let stats = aggregate(&numbers(), rect(0, 1, 0, 0), &[0, 1]).expect("numeric");
        assert!((stats.count - 2.0).abs() < f64::EPSILON);
        assert!((stats.sum - 4.0).abs() < f64::EPSILON);
        assert!((stats.average - 2.0).abs() < f64::EPSILON);
        assert!((stats.minimum - 1.0).abs() < f64::EPSILON);
        assert!((stats.maximum - 3.0).abs() < f64::EPSILON);
    }

    /// Decimals ride as text to keep their precision; the aggregate has to parse them back.
    #[test]
    fn numeric_columns_that_arrive_as_text_still_count() {
        let stats = aggregate(&numbers(), rect(0, 1, 1, 1), &[0, 1]).expect("numeric");
        assert!((stats.sum - 31.0).abs() < f64::EPSILON);
    }

    /// One non-numeric cell and there is no answer — a number under a column of names is worse
    /// than no number.
    #[test]
    fn a_single_non_numeric_cell_abandons_the_whole_selection() {
        let rows = ResultSet::new(
            vec![Column::new("n", "INT4"), Column::new("name", "VARCHAR")],
            vec![vec![Cell::Int(1), Cell::Text("bob".into())]],
        );
        assert!(aggregate(&rows, rect(0, 0, 0, 1), &[0]).is_none());
    }

    /// A numeric-looking string in a text column is not a number: postcodes do not average.
    #[test]
    fn a_number_in_a_text_column_does_not_count() {
        let rows = ResultSet::new(
            vec![Column::new("postcode", "VARCHAR")],
            vec![vec![Cell::Text("90210".into())]],
        );
        assert!(aggregate(&rows, rect(0, 0, 0, 0), &[0]).is_none());
    }

    #[test]
    fn a_null_is_not_a_number() {
        let rows = ResultSet::new(vec![Column::new("n", "INT4")], vec![vec![Cell::Null]]);
        assert!(aggregate(&rows, rect(0, 0, 0, 0), &[0]).is_none());
    }

    /// A rect holds display positions, so a filtered table aggregates the rows on screen.
    #[test]
    fn the_rect_is_read_through_the_visible_order() {
        let rows = ResultSet::new(
            vec![Column::new("n", "INT4")],
            vec![vec![Cell::Int(1)], vec![Cell::Int(2)], vec![Cell::Int(3)]],
        );
        // Only rows 2 and 0 are on screen, in that order.
        let stats = aggregate(&rows, rect(0, 1, 0, 0), &[2, 0]).expect("numeric");
        assert!((stats.sum - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn numbers_are_grouped_and_trimmed() {
        assert_eq!(format_number(1234.0), "1,234");
        assert_eq!(format_number(1_234_567.0), "1,234,567");
        assert_eq!(format_number(12.5), "12.5");
        assert_eq!(format_number(-1234.25), "-1,234.25");
        assert_eq!(format_number(0.0), "0");
    }

    /// Four decimals at most, matching `maximumFractionDigits: 4`.
    #[test]
    fn long_fractions_are_rounded_not_printed_whole() {
        assert_eq!(format_number(1.0 / 3.0), "0.3333");
        assert_eq!(format_number(2.000_05), "2.0001");
    }
}
