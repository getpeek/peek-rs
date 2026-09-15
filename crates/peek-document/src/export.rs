//! Serialising a [`ResultSet`] for export, and the filename it lands under.
//!
//! Ported from `~/labs/peek/src/tools/export/{csv,json}.ts` and `exportFilename.ts`. Pure
//! functions on purpose: the interesting cases — NULL, an embedded quote, a JSON column, a
//! query that slugifies to nothing — are all provable without a window or a database.
//!
//! Two deliberate departures from the reference, both where it throws rather than decides:
//! it derives its CSV header from `result[0]` and its rows from the same array, so an empty
//! result is a `TypeError` in both serialisers. [`ResultSet`] knows its columns independently
//! of its rows, so an empty set writes a header (CSV) or `[]` (JSON) instead of failing.

use serde_json::Value;

use crate::result::{Cell, ResultSet};

/// The reference joins CSV fields with `;`, not `,` — every field is quoted anyway, so this is
/// about what Excel opens without an import wizard in a comma-decimal locale.
const SEPARATOR: char = ';';

/// `exportFilename.ts` slices the slug to 40 characters.
const SLUG_LIMIT: usize = 40;

/// The set as CSV: one header row of column names, then one quoted field per cell.
#[must_use]
pub fn to_csv(result: &ResultSet) -> String {
    if result.column_count() == 0 {
        return String::new();
    }

    // The header is joined **raw** while every data field below is quoted. That asymmetry is
    // the reference's (`[headers, ...rows].map(row => row.join(";"))`) and is deliberately kept:
    // a user who exports the same result from both apps and diffs the files must get the same
    // bytes, the same reason the on-disk formats are frozen.
    //
    // The fragility that buys: a column whose name contains `;`, `"` or a newline — an aliased
    // expression can — writes a header no CSV parser reads back correctly. Quoting these would
    // fix it and diverge from the TypeScript app; do not change one without the other.
    let mut out = String::new();
    let mut names = result.columns().iter().map(|column| column.name.as_str());
    if let Some(first) = names.next() {
        out.push_str(first);
    }
    for name in names {
        out.push(SEPARATOR);
        out.push_str(name);
    }

    for row in result.rows() {
        out.push('\n');
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                out.push(SEPARATOR);
            }
            push_csv_field(&mut out, cell);
        }
    }
    out
}

/// The set as a compact JSON array of `{ column: value }` objects.
#[must_use]
pub fn to_json(result: &ResultSet) -> String {
    let rows: Vec<Value> = (0..result.row_count())
        .filter_map(|row| result.row_object(row))
        .collect();
    Value::Array(rows).to_string()
}

/// A filename for a query's export: the SQL slugified to at most 40 characters.
///
/// The reference asks Ollama for a descriptive name first and falls back to this; only the
/// fallback is ported, so an export never waits on a model that may not be running.
#[must_use]
pub fn filename(sql: &str, extension: &str) -> String {
    let slug = slug(sql);
    let base = if slug.is_empty() { "export" } else { &slug };
    format!("{base}.{extension}")
}

/// Lowercase, every run of non-alphanumerics collapsed to one `_`, trimmed, then cut to 40.
///
/// Trimming before the cut is the reference's order, so a slug may still end in `_` when the
/// cut lands mid-separator.
///
/// Public because the context menus name their exports after the scope as well as the query
/// (`orders-3-rows.csv`), so they need the base without an extension on it.
#[must_use]
pub fn slug(sql: &str) -> String {
    let mut collapsed = String::with_capacity(sql.len());
    let mut in_separator = false;
    for character in sql.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            collapsed.push(character);
            in_separator = false;
        } else if !in_separator {
            collapsed.push('_');
            in_separator = true;
        }
    }
    collapsed
        .trim_matches('_')
        .chars()
        .take(SLUG_LIMIT)
        .collect()
}

/// Every field is quoted and inner quotes are doubled, which is what keeps an embedded
/// newline, separator or JSON payload from terminating the field early.
///
/// NULL writes the word `null`, matching `String(value)` in the reference. That differs from
/// [`Cell::to_display_string`], which blanks it for a TSV copy — an export is read back by a
/// machine, and a blank field is indistinguishable from an empty string.
fn push_csv_field(out: &mut String, cell: &Cell) {
    out.push('"');
    match cell.to_json() {
        Value::String(text) => push_escaped(out, &text),
        other => push_escaped(out, &other.to_string()),
    }
    out.push('"');
}

fn push_escaped(out: &mut String, text: &str) {
    for character in text.chars() {
        if character == '"' {
            out.push('"');
        }
        out.push(character);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{filename, to_csv, to_json};
    use crate::result::{Cell, Column, ResultSet};

    fn sample() -> ResultSet {
        ResultSet::new(
            vec![
                Column::new("id", "INT4"),
                Column::new("name", "VARCHAR"),
                Column::new("meta", "JSONB"),
            ],
            vec![
                vec![
                    Cell::Int(1),
                    Cell::Text("bob".into()),
                    Cell::Json(json!({ "a": 1 })),
                ],
                vec![Cell::Int(2), Cell::Null, Cell::Json(json!([1, 2]))],
            ],
        )
    }

    #[test]
    fn csv_writes_a_bare_header_then_quoted_fields() {
        assert_eq!(
            to_csv(&sample()),
            concat!(
                "id;name;meta\n",
                "\"1\";\"bob\";\"{\"\"a\"\":1}\"\n",
                "\"2\";\"null\";\"[1,2]\""
            )
        );
    }

    /// `String(null)` in the reference. A blank field would be indistinguishable from an
    /// empty string once the file is read back.
    #[test]
    fn csv_writes_null_as_the_word_null() {
        let set = ResultSet::new(
            vec![Column::new("value", "TEXT")],
            vec![vec![Cell::Null], vec![Cell::Text(String::new())]],
        );
        assert_eq!(to_csv(&set), "value\n\"null\"\n\"\"");
    }

    /// A decode failure has no on-disk spelling of its own, so it exports as NULL does.
    #[test]
    fn csv_writes_an_undecodable_cell_as_null() {
        let set = ResultSet::new(
            vec![Column::new("blob", "BYTEA")],
            vec![vec![Cell::Undecodable]],
        );
        assert_eq!(to_csv(&set), "blob\n\"null\"");
    }

    /// Quoting every field is what makes these safe; without it the first one ends the row
    /// and the second ends the field.
    #[test]
    fn csv_survives_embedded_newlines_quotes_and_separators() {
        let set = ResultSet::new(
            vec![Column::new("note", "TEXT")],
            vec![
                vec![Cell::Text("line one\nline two".into())],
                vec![Cell::Text("say \"hi\"".into())],
                vec![Cell::Text("a;b,c".into())],
            ],
        );
        assert_eq!(
            to_csv(&set),
            "note\n\"line one\nline two\"\n\"say \"\"hi\"\"\"\n\"a;b,c\""
        );
    }

    #[test]
    fn csv_of_a_set_with_no_columns_is_empty() {
        assert_eq!(to_csv(&ResultSet::default()), String::new());
    }

    #[test]
    fn csv_of_a_set_with_no_rows_is_just_the_header() {
        let set = ResultSet::new(vec![Column::new("id", "INT4")], Vec::new());
        assert_eq!(to_csv(&set), "id");
    }

    #[test]
    fn json_writes_one_compact_object_per_row() {
        assert_eq!(
            to_json(&sample()),
            r#"[{"id":1,"name":"bob","meta":{"a":1}},{"id":2,"name":null,"meta":[1,2]}]"#
        );
    }

    #[test]
    fn json_of_an_empty_set_is_an_empty_array() {
        assert_eq!(to_json(&ResultSet::default()), "[]");
    }

    #[test]
    fn a_filename_slugifies_the_query() {
        assert_eq!(
            filename("SELECT * FROM users WHERE id = 1", "csv"),
            "select_from_users_where_id_1.csv"
        );
    }

    /// Leading and trailing separators go before the cut, so `;` at the end of a statement
    /// never shows up in the name.
    #[test]
    fn a_filename_trims_its_separators() {
        assert_eq!(filename("  select 1;  ", "json"), "select_1.json");
    }

    #[test]
    fn a_filename_is_cut_to_forty_characters() {
        let name = filename(
            "select a_very_long_column_name, another_very_long_one from a_table",
            "csv",
        );
        assert_eq!(name, "select_a_very_long_column_name_another_v.csv");
        assert_eq!(name.len(), 40 + ".csv".len());
    }

    #[test]
    fn a_query_that_slugifies_to_nothing_falls_back_to_export() {
        assert_eq!(filename("   ***   ", "csv"), "export.csv");
        assert_eq!(filename("", "json"), "export.json");
    }
}
