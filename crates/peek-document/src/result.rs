//! Cell values and the columnar result set.
//!
//! The reference hands the frontend `[[column_name, value, column_type], …]` per row
//! (`~/labs/peek/src-tauri/src/database/mod.rs`), repeating the name and type in every cell — on
//! the 6,515-row results this codebase has to open, that is three allocations per cell for two
//! facts that belong to the column. [`ResultSet`] keeps them once.
//!
//! The triple form is still the frozen on-disk shape of the results sidecar, so
//! [`ResultSet::from_sidecar_rows`] and [`ResultSet::to_sidecar_rows`] convert both ways.

use std::fmt;

use serde_json::{Map, Value};

/// One column of a result, named and typed once for the whole set.
///
/// `sql_type` is the driver's own spelling (`INT4`, `VARCHAR`, `TIMESTAMPTZ`, `UNSIGNED BIGINT`),
/// uppercase by sqlx convention, and is what every type predicate in the UI matches on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub sql_type: String,
}

impl Column {
    #[must_use]
    pub fn new(name: impl Into<String>, sql_type: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sql_type: sql_type.into(),
        }
    }
}

/// One cell.
///
/// `NUMERIC`/`DECIMAL` deliberately arrive as [`Cell::Text`]: the reference reads them through
/// `rust_decimal` and serialises to a string to keep the precision a JSON double would lose, and
/// the aggregation and chart code downstream parses them back. Changing that silently truncates
/// money columns.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Text, and anything pre-stringified by the driver: dates, timestamps, uuids, decimals.
    Text(String),
    Json(Value),
    /// The driver produced bytes this column's decoder could not read.
    ///
    /// The reference cannot express this: every arm is `try_get(..).map_or(Value::Null, ..)`, so
    /// a decode failure and a real `NULL` are the same value and a column of unreadable data
    /// reads as an empty one. Keeping them apart lets the table say which it is.
    Undecodable,
}

impl Cell {
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The cell as the sidecar's JSON. [`Cell::Undecodable`] writes as `null`, because the
    /// on-disk format has no way to say anything else and the TypeScript app must still read it.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Null | Self::Undecodable => Value::Null,
            Self::Bool(value) => Value::Bool(*value),
            Self::Int(value) => Value::from(*value),
            Self::Float(value) => {
                serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number)
            }
            Self::Text(value) => Value::String(value.clone()),
            Self::Json(value) => value.clone(),
        }
    }

    /// A cell read back from the sidecar, where the only type information is the JSON shape.
    #[must_use]
    pub fn from_json(value: Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(flag) => Self::Bool(flag),
            Value::String(text) => Self::Text(text),
            Value::Number(number) => number.as_i64().map_or_else(
                || number.as_f64().map_or(Self::Null, Self::Float),
                Self::Int,
            ),
            other @ (Value::Array(_) | Value::Object(_)) => Self::Json(other),
        }
    }

    /// The cell as the UI shows and copies it: `stringifyValue` from
    /// `~/labs/peek/src/canvas/nodes/Result/stringify.ts`, where null is the empty string so a
    /// TSV copy leaves the field blank rather than writing the word "null".
    #[must_use]
    pub fn to_display_string(&self) -> String {
        match self {
            Self::Null | Self::Undecodable => String::new(),
            Self::Bool(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Text(value) => value.clone(),
            Self::Json(value) => value.to_string(),
        }
    }
}

impl fmt::Display for Cell {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_display_string())
    }
}

/// A query's rows, stored column-major in its header and row-major in its data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultSet {
    columns: Vec<Column>,
    rows: Vec<Vec<Cell>>,
}

impl ResultSet {
    /// # Panics
    /// Never; rows wider or narrower than `columns` are padded or truncated to fit, so the set
    /// is always rectangular and `row[i]` always belongs to `columns[i]`.
    #[must_use]
    pub fn new(columns: Vec<Column>, rows: Vec<Vec<Cell>>) -> Self {
        let width = columns.len();
        let rows = rows
            .into_iter()
            .map(|mut row| {
                row.resize(width, Cell::Null);
                row
            })
            .collect();
        Self { columns, rows }
    }

    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    #[must_use]
    pub fn rows(&self) -> &[Vec<Cell>] {
        &self.rows
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[must_use]
    pub fn cell(&self, row: usize, column: usize) -> Option<&Cell> {
        self.rows.get(row)?.get(column)
    }

    #[must_use]
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|column| column.name == name)
    }

    /// The sidecar's `[[name, value, type], …]` per row.
    ///
    /// An empty set writes `[]`, which is why the reference derives its headers from `data[0]`
    /// and has no column list at all: a result with no rows has no columns on disk either.
    #[must_use]
    pub fn to_sidecar_rows(&self) -> Value {
        let rows = self
            .rows
            .iter()
            .map(|row| {
                let cells = row
                    .iter()
                    .zip(&self.columns)
                    .map(|(cell, column)| {
                        Value::Array(vec![
                            Value::String(column.name.clone()),
                            cell.to_json(),
                            Value::String(column.sql_type.clone()),
                        ])
                    })
                    .collect();
                Value::Array(cells)
            })
            .collect();
        Value::Array(rows)
    }

    /// Reads the sidecar form back, taking the columns from the first row as the reference does.
    ///
    /// Anything that is not an array of arrays of `[name, value, type]` yields an empty set
    /// rather than an error: the sidecar is a cache, and a malformed one must not stop a
    /// document from opening.
    #[must_use]
    pub fn from_sidecar_rows(value: &Value) -> Self {
        let Some(rows) = value.as_array() else {
            return Self::default();
        };
        let Some(first) = rows.first().and_then(Value::as_array) else {
            return Self::default();
        };
        let columns: Vec<Column> = first.iter().filter_map(sidecar_column).collect();
        let width = columns.len();
        let rows = rows
            .iter()
            .map(|row| {
                let mut cells: Vec<Cell> = row
                    .as_array()
                    .map(|cells| {
                        cells
                            .iter()
                            .map(|cell| {
                                cell.as_array()
                                    .and_then(|triple| triple.get(1))
                                    .cloned()
                                    .map_or(Cell::Null, Cell::from_json)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                cells.resize(width, Cell::Null);
                cells
            })
            .collect();
        Self { columns, rows }
    }

    /// One row as a `{ column: value }` object, the shape the JSON export writes.
    #[must_use]
    pub fn row_object(&self, row: usize) -> Option<Value> {
        let cells = self.rows.get(row)?;
        let mut object = Map::new();
        for (column, cell) in self.columns.iter().zip(cells) {
            object.insert(column.name.clone(), cell.to_json());
        }
        Some(Value::Object(object))
    }
}

fn sidecar_column(cell: &Value) -> Option<Column> {
    let triple = cell.as_array()?;
    let name = triple.first()?.as_str()?;
    let sql_type = triple.get(2).and_then(Value::as_str).unwrap_or_default();
    Some(Column::new(name, sql_type))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Cell, Column, ResultSet};

    fn sample() -> ResultSet {
        ResultSet::new(
            vec![Column::new("id", "INT4"), Column::new("name", "VARCHAR")],
            vec![
                vec![Cell::Int(1), Cell::Text("bob".into())],
                vec![Cell::Int(2), Cell::Null],
            ],
        )
    }

    #[test]
    fn sidecar_form_round_trips() {
        let set = sample();
        let json = set.to_sidecar_rows();
        assert_eq!(
            json,
            json!([
                [["id", 1, "INT4"], ["name", "bob", "VARCHAR"]],
                [["id", 2, "INT4"], ["name", null, "VARCHAR"]]
            ])
        );
        assert_eq!(ResultSet::from_sidecar_rows(&json), set);
    }

    /// A result with no rows has no columns on disk, because the reference derives headers from
    /// `data[0]`. Round-tripping an empty set must therefore not invent any.
    #[test]
    fn an_empty_set_survives_the_sidecar() {
        let empty = ResultSet::new(vec![Column::new("id", "INT4")], vec![]);
        let json = empty.to_sidecar_rows();
        assert_eq!(json, json!([]));
        assert_eq!(ResultSet::from_sidecar_rows(&json), ResultSet::default());
    }

    #[test]
    fn a_malformed_sidecar_reads_as_empty_rather_than_failing() {
        for broken in [json!(null), json!({}), json!("nonsense"), json!([3, 4])] {
            assert!(ResultSet::from_sidecar_rows(&broken).is_empty());
        }
    }

    /// Losing this distinction is what makes a column of unreadable bytes look like a column of
    /// NULLs, which is the reference's behaviour and the reason for the variant.
    #[test]
    fn undecodable_is_not_null_in_memory_but_is_null_on_disk() {
        assert!(!Cell::Undecodable.is_null());
        assert_ne!(Cell::Undecodable, Cell::Null);
        assert_eq!(Cell::Undecodable.to_json(), json!(null));
    }

    /// `stringifyValue` renders null as the empty string so a spreadsheet paste leaves the cell
    /// blank instead of spelling out "null".
    #[test]
    fn display_matches_stringify_value() {
        assert_eq!(Cell::Null.to_display_string(), "");
        assert_eq!(Cell::Undecodable.to_display_string(), "");
        assert_eq!(Cell::Bool(true).to_display_string(), "true");
        assert_eq!(Cell::Text("x".into()).to_display_string(), "x");
        assert_eq!(
            Cell::Json(json!({"a": 1})).to_display_string(),
            r#"{"a":1}"#
        );
    }

    /// Decimals ride as text; reading them back as a float would be the precision loss the
    /// string form exists to prevent.
    #[test]
    fn decimal_text_stays_text_through_the_sidecar() {
        let set = ResultSet::new(
            vec![Column::new("amount", "NUMERIC")],
            vec![vec![Cell::Text("0.10000000000000000001".into())]],
        );
        let back = ResultSet::from_sidecar_rows(&set.to_sidecar_rows());
        assert_eq!(
            back.cell(0, 0),
            Some(&Cell::Text("0.10000000000000000001".into()))
        );
    }

    #[test]
    fn rows_are_padded_to_the_column_count() {
        let set = ResultSet::new(
            vec![Column::new("a", "INT4"), Column::new("b", "INT4")],
            vec![vec![Cell::Int(1)]],
        );
        assert_eq!(set.cell(0, 1), Some(&Cell::Null));
    }

    #[test]
    fn row_object_is_the_json_export_shape() {
        assert_eq!(
            sample().row_object(0),
            Some(json!({"id": 1, "name": "bob"}))
        );
    }
}
