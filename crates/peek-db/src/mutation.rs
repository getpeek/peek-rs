//! Generating the statements the Result node's inline editing runs.
//!
//! Ported from `~/labs/peek/src/canvas/nodes/Result/cell/inlineEdit.ts`. This is the one place
//! that builds SQL from user data, so every identifier goes through [`Engine::quote_identifier`]
//! and every value through [`format_literal`].
//!
//! These statements are **not** undoable: they change the database, not the document, so the
//! canvas history never sees them.

use std::fmt;

use crate::engine::Engine;
use peek_document as sql_type;
use peek_document::Cell;

/// Why a result cannot be edited in place.
///
/// The reference builds these as English strings at the call site; naming them means the UI can
/// phrase them and the tests can assert on them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotEditable {
    /// Not a single-table `SELECT`: a join, an aggregate, or a write.
    NotASingleTableSelect,
    NoPrimaryKey {
        table: String,
    },
    /// The row on screen does not carry every primary-key column, so no `WHERE` can name it.
    MissingPrimaryKeyColumns {
        columns: Vec<String>,
    },
}

impl fmt::Display for NotEditable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotASingleTableSelect => {
                write!(formatter, "Cannot edit: query is not a single-table SELECT")
            }
            Self::NoPrimaryKey { table } => {
                write!(formatter, "Cannot edit: no primary key on \"{table}\"")
            }
            Self::MissingPrimaryKeyColumns { columns } => write!(
                formatter,
                "Row is missing primary key columns ({})",
                columns.join(", ")
            ),
        }
    }
}

impl std::error::Error for NotEditable {}

/// One primary-key column bound to the literal identifying a specific row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub column: String,
    pub literal: String,
}

/// Renders a value as a SQL literal of the given column type.
///
/// The empty string is `NULL`, which is how the NULL button in every cell editor works: it
/// commits an empty draft rather than carrying a separate flag.
#[must_use]
pub fn format_literal(value: &str, sql_type: &str, engine: Engine) -> String {
    if value.is_empty() {
        return "NULL".to_string();
    }
    if sql_type::is_boolean(sql_type) {
        return boolean_literal(value);
    }
    if sql_type::is_json(sql_type) {
        return json_literal(value, sql_type, engine);
    }
    if sql_type::is_numeric(sql_type) {
        // Anything that does not parse falls through to a quoted string rather than being
        // inlined bare: the reference throws here, but a malformed number must not be able to
        // reach the statement unquoted.
        if value.parse::<f64>().is_ok_and(f64::is_finite) {
            return value.to_string();
        }
        return quote_literal(value);
    }
    quote_literal(value)
}

/// The literal for a cell already in hand, used to pin a row by its primary key.
#[must_use]
pub fn format_cell_literal(cell: &Cell, sql_type: &str, engine: Engine) -> String {
    match cell {
        Cell::Null | Cell::Undecodable => "NULL".to_string(),
        Cell::Bool(flag) => {
            if *flag {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        other => format_literal(&other.to_display_string(), sql_type, engine),
    }
}

/// `TRUE`/`FALSE` from the several spellings a driver or an editor can produce; anything else is
/// `NULL`, matching the reference.
fn boolean_literal(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "true" | "t" | "1" => "TRUE".to_string(),
        "false" | "f" | "0" => "FALSE".to_string(),
        _ => "NULL".to_string(),
    }
}

/// `MySQL` has no cast syntax for JSON columns and takes a plain string literal; Postgres needs
/// the cast or it will not accept the parameter at all.
fn json_literal(value: &str, sql_type: &str, engine: Engine) -> String {
    let quoted = quote_literal(value);
    match engine {
        Engine::MySql => quoted,
        Engine::Postgres | Engine::Unknown => {
            if sql_type.eq_ignore_ascii_case("JSONB") {
                format!("{quoted}::jsonb")
            } else {
                format!("{quoted}::json")
            }
        }
    }
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `UPDATE <table> SET <column> = <literal> WHERE <pk> = <literal> AND …`
///
/// # Errors
/// Returns [`NotEditable::NoPrimaryKey`] when there is nothing to put in the `WHERE`; without it
/// the statement would rewrite every row in the table.
pub fn build_update(
    engine: Engine,
    target: (&str, &str),
    new_literal: &str,
    keys: &[KeyBinding],
) -> Result<String, NotEditable> {
    let (table, column) = target;
    if keys.is_empty() {
        return Err(NotEditable::NoPrimaryKey {
            table: table.to_string(),
        });
    }
    let where_clause = keys
        .iter()
        .map(|key| format!("{} = {}", engine.quote_identifier(&key.column), key.literal))
        .collect::<Vec<_>>()
        .join(" AND ");
    Ok(format!(
        "UPDATE {} SET {} = {new_literal} WHERE {where_clause}",
        engine.quote_identifier(table),
        engine.quote_identifier(column),
    ))
}

/// `DELETE FROM <table> WHERE <pk> IN (…)`, or the tuple form for a composite key.
///
/// # Errors
/// Returns [`NotEditable::NoPrimaryKey`] with no key columns, and
/// [`NotEditable::MissingPrimaryKeyColumns`] when a row does not carry one of them — either
/// would otherwise produce a `DELETE` with no `WHERE`.
pub fn build_delete(
    engine: Engine,
    table: &str,
    key_columns: &[String],
    rows: &[Vec<KeyBinding>],
) -> Result<String, NotEditable> {
    if key_columns.is_empty() {
        return Err(NotEditable::NoPrimaryKey {
            table: table.to_string(),
        });
    }
    if rows.is_empty() {
        return Err(NotEditable::MissingPrimaryKeyColumns {
            columns: key_columns.to_vec(),
        });
    }
    let quoted_table = engine.quote_identifier(table);

    if let [only] = key_columns {
        let literals = rows
            .iter()
            .map(|row| literal_for(row, only))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");
        return Ok(format!(
            "DELETE FROM {quoted_table} WHERE {} IN ({literals})",
            engine.quote_identifier(only),
        ));
    }

    let tuple = key_columns
        .iter()
        .map(|column| engine.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let values = rows
        .iter()
        .map(|row| {
            let literals = key_columns
                .iter()
                .map(|column| literal_for(row, column))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            Ok(format!("({literals})"))
        })
        .collect::<Result<Vec<_>, NotEditable>>()?
        .join(", ");
    Ok(format!(
        "DELETE FROM {quoted_table} WHERE ({tuple}) IN ({values})"
    ))
}

fn literal_for(row: &[KeyBinding], column: &str) -> Result<String, NotEditable> {
    row.iter()
        .find(|key| key.column == column)
        .map(|key| key.literal.clone())
        .ok_or_else(|| NotEditable::MissingPrimaryKeyColumns {
            columns: vec![column.to_string()],
        })
}

#[cfg(test)]
mod tests {
    use super::{
        KeyBinding, NotEditable, build_delete, build_update, format_cell_literal, format_literal,
    };
    use crate::engine::Engine;
    use peek_document::Cell;

    fn key(column: &str, literal: &str) -> KeyBinding {
        KeyBinding {
            column: column.to_string(),
            literal: literal.to_string(),
        }
    }

    #[test]
    fn text_literals_are_quoted_and_escaped() {
        assert_eq!(
            format_literal("O'Brien", "VARCHAR", Engine::Postgres),
            "'O''Brien'"
        );
    }

    /// The NULL button commits an empty draft rather than carrying its own flag.
    #[test]
    fn the_empty_string_is_null() {
        assert_eq!(format_literal("", "VARCHAR", Engine::Postgres), "NULL");
        assert_eq!(format_literal("", "INT4", Engine::Postgres), "NULL");
    }

    #[test]
    fn numbers_are_inlined_bare() {
        assert_eq!(format_literal("42", "INT4", Engine::Postgres), "42");
        assert_eq!(format_literal("-1.5", "FLOAT8", Engine::Postgres), "-1.5");
    }

    /// The reference throws on a non-numeric draft. Quoting instead keeps a malformed value from
    /// ever reaching the statement unquoted; the database rejects it with a type error.
    #[test]
    fn a_malformed_number_is_quoted_not_inlined() {
        assert_eq!(
            format_literal("1; drop table users", "INT4", Engine::Postgres),
            "'1; drop table users'"
        );
        assert_eq!(format_literal("NaN", "FLOAT8", Engine::Postgres), "'NaN'");
    }

    #[test]
    fn booleans_accept_the_several_spellings() {
        for truthy in ["true", "TRUE", "t", "1"] {
            assert_eq!(format_literal(truthy, "BOOL", Engine::Postgres), "TRUE");
        }
        for falsy in ["false", "F", "0"] {
            assert_eq!(format_literal(falsy, "BOOL", Engine::Postgres), "FALSE");
        }
        assert_eq!(format_literal("maybe", "BOOL", Engine::Postgres), "NULL");
    }

    /// `MySQL` has no cast syntax here; Postgres needs one and distinguishes json from jsonb.
    #[test]
    fn json_literals_are_cast_only_on_postgres() {
        assert_eq!(
            format_literal(r#"{"a":1}"#, "JSONB", Engine::Postgres),
            r#"'{"a":1}'::jsonb"#
        );
        assert_eq!(
            format_literal(r#"{"a":1}"#, "JSON", Engine::Postgres),
            r#"'{"a":1}'::json"#
        );
        assert_eq!(
            format_literal(r#"{"a":1}"#, "JSON", Engine::MySql),
            r#"'{"a":1}'"#
        );
    }

    #[test]
    fn update_names_the_row_by_its_key() {
        let sql = build_update(
            Engine::Postgres,
            ("users", "name"),
            "'bob'",
            &[key("id", "7")],
        )
        .unwrap();
        assert_eq!(sql, r#"UPDATE "users" SET "name" = 'bob' WHERE "id" = 7"#);
    }

    #[test]
    fn update_ands_a_composite_key() {
        let sql = build_update(
            Engine::MySql,
            ("memberships", "role"),
            "'admin'",
            &[key("user_id", "1"), key("org_id", "2")],
        )
        .unwrap();
        assert_eq!(
            sql,
            "UPDATE `memberships` SET `role` = 'admin' WHERE `user_id` = 1 AND `org_id` = 2"
        );
    }

    /// Without this guard the statement is `UPDATE users SET name = 'bob'` — every row.
    #[test]
    fn update_refuses_to_build_without_a_key() {
        assert_eq!(
            build_update(Engine::Postgres, ("users", "name"), "'bob'", &[]),
            Err(NotEditable::NoPrimaryKey {
                table: "users".into()
            })
        );
    }

    #[test]
    fn delete_uses_in_for_a_single_key() {
        let sql = build_delete(
            Engine::Postgres,
            "users",
            &["id".to_string()],
            &[vec![key("id", "1")], vec![key("id", "2")]],
        )
        .unwrap();
        assert_eq!(sql, r#"DELETE FROM "users" WHERE "id" IN (1, 2)"#);
    }

    #[test]
    fn delete_uses_tuples_for_a_composite_key() {
        let sql = build_delete(
            Engine::Postgres,
            "memberships",
            &["user_id".to_string(), "org_id".to_string()],
            &[
                vec![key("user_id", "1"), key("org_id", "2")],
                vec![key("org_id", "4"), key("user_id", "3")],
            ],
        )
        .unwrap();
        // The second row lists its keys in the other order; the tuple follows `key_columns`.
        assert_eq!(
            sql,
            r#"DELETE FROM "memberships" WHERE ("user_id", "org_id") IN ((1, 2), (3, 4))"#
        );
    }

    #[test]
    fn delete_refuses_a_row_missing_a_key_column() {
        let error = build_delete(
            Engine::Postgres,
            "memberships",
            &["user_id".to_string(), "org_id".to_string()],
            &[vec![key("user_id", "1")]],
        )
        .unwrap_err();
        assert_eq!(
            error,
            NotEditable::MissingPrimaryKeyColumns {
                columns: vec!["org_id".to_string()]
            }
        );
    }

    #[test]
    fn delete_refuses_to_build_without_a_key_or_rows() {
        assert!(build_delete(Engine::Postgres, "users", &[], &[vec![key("id", "1")]]).is_err());
        assert!(build_delete(Engine::Postgres, "users", &["id".to_string()], &[]).is_err());
    }

    /// A key column that is NULL still produces the literal `NULL`, which matches no row — the
    /// statement is then a harmless no-op rather than a wildcard.
    #[test]
    fn a_null_key_cell_is_a_null_literal() {
        assert_eq!(
            format_cell_literal(&Cell::Null, "INT4", Engine::Postgres),
            "NULL"
        );
        assert_eq!(
            format_cell_literal(&Cell::Undecodable, "INT4", Engine::Postgres),
            "NULL"
        );
    }

    #[test]
    fn cell_literals_keep_decimal_precision() {
        let cell = Cell::Text("0.10000000000000000001".into());
        assert_eq!(
            format_cell_literal(&cell, "NUMERIC", Engine::Postgres),
            "0.10000000000000000001"
        );
    }

    #[test]
    fn a_quoted_table_name_cannot_break_out() {
        let sql = build_update(
            Engine::Postgres,
            (r#"users"; drop table x; --"#, "name"),
            "'bob'",
            &[key("id", "1")],
        )
        .unwrap();
        assert!(sql.starts_with(r#"UPDATE "users""; drop table x; --" SET "#));
    }
}
