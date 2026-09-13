//! Classification of a driver's type spelling.
//!
//! Ported from `~/labs/peek/src/canvas/nodes/Result/cell/inlineEdit.ts`, which matches the
//! **whole uppercased name** against fixed sets. That is a different question from
//! `TableDefinition`'s eight colour buckets, which match a schema's `udt_name` and tolerate a
//! parameter list (`numeric(10,2)`); these names come from `col.type_info().name()` and never
//! carry one, so an exact match is right here and a prefix match would be wrong.

/// Every spelling `isNumericType` accepts. `DECIMAL`/`NUMERIC` are in the list even though they
/// arrive as strings — that is precisely why the aggregate code has to parse them.
const NUMERIC: &[&str] = &[
    "INT2",
    "INT4",
    "INT8",
    "INT",
    "SMALLINT",
    "MEDIUMINT",
    "BIGINT",
    "TINYINT",
    "FLOAT4",
    "FLOAT8",
    "FLOAT",
    "DOUBLE",
    "DECIMAL",
    "NUMERIC",
];

/// Long-form text only: `VARCHAR`/`CHAR` stay single-line, these get a multiline editor.
const TEXT: &[&str] = &["TEXT", "TINYTEXT", "MEDIUMTEXT", "LONGTEXT"];

const TIMESTAMP: &[&str] = &["TIMESTAMP", "TIMESTAMPTZ", "DATETIME"];

fn matches(sql_type: &str, names: &[&str]) -> bool {
    names.iter().any(|name| sql_type.eq_ignore_ascii_case(name))
}

#[must_use]
pub fn is_boolean(sql_type: &str) -> bool {
    sql_type.eq_ignore_ascii_case("BOOL") || sql_type.eq_ignore_ascii_case("BOOLEAN")
}

#[must_use]
pub fn is_numeric(sql_type: &str) -> bool {
    matches(sql_type, NUMERIC)
}

#[must_use]
pub fn is_uuid(sql_type: &str) -> bool {
    sql_type.eq_ignore_ascii_case("UUID")
}

/// Long-form text that earns a multiline editor.
#[must_use]
pub fn is_text(sql_type: &str) -> bool {
    matches(sql_type, TEXT)
}

#[must_use]
pub fn is_timestamp(sql_type: &str) -> bool {
    matches(sql_type, TIMESTAMP)
}

#[must_use]
pub fn is_json(sql_type: &str) -> bool {
    sql_type.eq_ignore_ascii_case("JSON") || sql_type.eq_ignore_ascii_case("JSONB")
}

/// How wide a freshly placed result node makes this column, from `columnWidthForType` in
/// `~/labs/peek/src/canvas/executeQueries.ts`. Summed across the first row, floored at
/// [`MIN_RESULT_WIDTH`], this is the node's width.
#[must_use]
pub fn placement_column_width(sql_type: &str) -> f64 {
    if is_uuid(sql_type) {
        UUID_COLUMN_WIDTH
    } else if is_timestamp(sql_type) {
        TIMESTAMP_COLUMN_WIDTH
    } else if is_numeric(sql_type) {
        NUMERIC_COLUMN_WIDTH
    } else {
        RESULT_COLUMN_WIDTH
    }
}

const RESULT_COLUMN_WIDTH: f64 = 250.0;
const UUID_COLUMN_WIDTH: f64 = 440.0;
const TIMESTAMP_COLUMN_WIDTH: f64 = 280.0;
const NUMERIC_COLUMN_WIDTH: f64 = 130.0;
pub const MIN_RESULT_WIDTH: f64 = 200.0;

#[cfg(test)]
mod tests {
    use super::{
        is_boolean, is_json, is_numeric, is_text, is_timestamp, is_uuid, placement_column_width,
    };

    #[test]
    fn postgres_and_mysql_spellings() {
        assert!(is_numeric("INT4"));
        assert!(is_numeric("BIGINT"));
        assert!(is_numeric("NUMERIC"));
        assert!(is_boolean("BOOL"));
        assert!(is_boolean("BOOLEAN"));
        assert!(is_uuid("UUID"));
        assert!(is_text("LONGTEXT"));
        assert!(is_timestamp("TIMESTAMPTZ"));
        assert!(is_json("JSONB"));
    }

    /// sqlx uppercases, but the sidecar is also read back from disk and from other tools.
    #[test]
    fn matching_ignores_case() {
        assert!(is_numeric("int4"));
        assert!(is_uuid("uuid"));
        assert!(is_json("jsonb"));
    }

    /// `VARCHAR` and `CHAR` are text, but not the long-form kind that gets a multiline editor —
    /// treating them as such would open a textarea for every name column in the database.
    #[test]
    fn varchar_is_not_long_form_text() {
        assert!(!is_text("VARCHAR"));
        assert!(!is_text("CHAR"));
        assert!(is_text("TEXT"));
    }

    /// Exact match, not prefix: `INT` must not swallow `INTERVAL`.
    #[test]
    fn a_longer_spelling_is_not_a_prefix_match() {
        assert!(!is_numeric("INTERVAL"));
        assert!(!is_boolean("BOOLEANISH"));
        assert!(!is_timestamp("TIMESTAMP_MS"));
    }

    #[test]
    fn placement_widths_match_execute_queries() {
        assert!((placement_column_width("UUID") - 440.0).abs() < f64::EPSILON);
        assert!((placement_column_width("TIMESTAMPTZ") - 280.0).abs() < f64::EPSILON);
        assert!((placement_column_width("INT4") - 130.0).abs() < f64::EPSILON);
        assert!((placement_column_width("VARCHAR") - 250.0).abs() < f64::EPSILON);
    }

    /// UUID is checked before numeric and timestamp, so the order of the predicates in
    /// `placement_column_width` is load-bearing.
    #[test]
    fn uuid_wins_over_the_generic_width() {
        assert!((placement_column_width("uuid") - 440.0).abs() < f64::EPSILON);
    }
}
