//! Whether a result can be edited in place, and how a row is named in SQL.
//!
//! Ported from `cell/inlineEdit.ts`. Every refusal is a named [`NotEditable`], because the user
//! has to be told *why* a cell will not open — "cannot edit" with no reason is the kind of dead
//! end that makes people stop trusting a grid.
//!
//! The safety argument for the whole feature lives here: an edit is only built when the query is
//! a single-table `SELECT`, the table has a primary key, and the row on screen carries every
//! key column. The resulting statement is pinned to one row by construction, which is why the
//! unbounded-write gate — which only flags `TRUNCATE` and a `DELETE` with no `WHERE` — has
//! nothing to say about it.

use peek_db::Engine;
use peek_db::mutation::{KeyBinding, NotEditable, format_cell_literal};
use peek_document::ResultSet;
use peek_lsp::{QueryInfo, StatementType};

/// The table a result can be edited through, if any.
///
/// A join, an aggregate, a `SELECT` over a CTE and every write statement all yield `None`: there
/// is no single row behind a cell to update.
pub(super) fn editable_table(info: &QueryInfo) -> Option<String> {
    if info.statement_type != StatementType::Select {
        return None;
    }
    let [table] = info.tables.as_slice() else {
        return None;
    };
    if table.is_joined {
        return None;
    }
    Some(table.name.clone())
}

/// Binds each primary-key column to the literal identifying one row.
///
/// # Errors
/// [`NotEditable::NoPrimaryKey`] when the table has none, and
/// [`NotEditable::MissingPrimaryKeyColumns`] when the result does not show them — a
/// `SELECT name FROM users` cannot say *which* user a row is.
pub(super) fn key_bindings(
    rows: &ResultSet,
    row: usize,
    keys: (&str, &[String]),
    engine: Engine,
) -> Result<Vec<KeyBinding>, NotEditable> {
    let (table, key_columns) = keys;
    if key_columns.is_empty() {
        return Err(NotEditable::NoPrimaryKey {
            table: table.to_string(),
        });
    }

    let mut bindings = Vec::with_capacity(key_columns.len());
    let mut missing = Vec::new();
    for column in key_columns {
        let Some(index) = rows.column_index(column) else {
            missing.push(column.clone());
            continue;
        };
        let Some(cell) = rows.cell(row, index) else {
            missing.push(column.clone());
            continue;
        };
        let sql_type = rows
            .columns()
            .get(index)
            .map_or("", |column| column.sql_type.as_str());
        bindings.push(KeyBinding {
            column: column.clone(),
            literal: format_cell_literal(cell, sql_type, engine),
        });
    }

    if missing.is_empty() {
        Ok(bindings)
    } else {
        Err(NotEditable::MissingPrimaryKeyColumns { columns: missing })
    }
}

#[cfg(test)]
mod tests {
    use peek_db::Engine;
    use peek_document::{Cell, Column, ResultSet};
    use peek_lsp::{QueryInfo, StatementType, TableRef};

    use super::{editable_table, key_bindings};

    fn info(statement: StatementType, tables: &[(&str, bool)]) -> QueryInfo {
        QueryInfo {
            statement_type: statement,
            tables: tables
                .iter()
                .map(|(name, joined)| TableRef {
                    name: (*name).to_string(),
                    alias: None,
                    is_joined: *joined,
                })
                .collect(),
        }
    }

    fn rows() -> ResultSet {
        ResultSet::new(
            vec![Column::new("id", "INT4"), Column::new("name", "VARCHAR")],
            vec![vec![Cell::Int(7), Cell::Text("bob".into())]],
        )
    }

    #[test]
    fn a_single_table_select_is_editable() {
        let editable = editable_table(&info(StatementType::Select, &[("users", false)]));
        assert_eq!(editable.as_deref(), Some("users"));
    }

    /// A join has no single row behind a cell, so there is nothing to pin an update to.
    #[test]
    fn a_join_is_not_editable() {
        assert!(editable_table(&info(StatementType::Select, &[("users", true)])).is_none());
        assert!(
            editable_table(&info(
                StatementType::Select,
                &[("users", false), ("orders", false)]
            ))
            .is_none()
        );
    }

    #[test]
    fn a_write_statement_is_not_editable() {
        assert!(editable_table(&info(StatementType::Update, &[("users", false)])).is_none());
        assert!(editable_table(&info(StatementType::Other, &[("users", false)])).is_none());
    }

    /// A query the parser could not read yields no tables, and so no editing.
    #[test]
    fn a_query_with_no_tables_is_not_editable() {
        assert!(editable_table(&info(StatementType::Select, &[])).is_none());
    }

    #[test]
    fn a_row_is_named_by_its_key_columns() {
        let keys = vec!["id".to_string()];
        let bound = key_bindings(&rows(), 0, ("users", &keys), Engine::Postgres).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].column, "id");
        assert_eq!(bound[0].literal, "7");
    }

    /// Without a key there is no `WHERE`, and an update would rewrite the table.
    #[test]
    fn a_table_without_a_primary_key_refuses() {
        let error = key_bindings(&rows(), 0, ("users", &[]), Engine::Postgres).unwrap_err();
        assert!(error.to_string().contains("no primary key"));
    }

    /// `SELECT name FROM users` cannot say which user a row is.
    #[test]
    fn a_result_that_hides_the_key_refuses_and_names_it() {
        let keys = vec!["id".to_string()];
        let without = ResultSet::new(
            vec![Column::new("name", "VARCHAR")],
            vec![vec![Cell::Text("bob".into())]],
        );
        let error = key_bindings(&without, 0, ("users", &keys), Engine::Postgres).unwrap_err();
        assert!(error.to_string().contains("id"), "{error}");
    }

    #[test]
    fn a_composite_key_binds_every_column_in_order() {
        let rows = ResultSet::new(
            vec![
                Column::new("org_id", "INT4"),
                Column::new("user_id", "INT4"),
                Column::new("role", "VARCHAR"),
            ],
            vec![vec![Cell::Int(1), Cell::Int(2), Cell::Text("admin".into())]],
        );
        let keys = vec!["org_id".to_string(), "user_id".to_string()];
        let bound = key_bindings(&rows, 0, ("memberships", &keys), Engine::Postgres).unwrap();
        assert_eq!(bound.len(), 2);
        assert_eq!(bound[0].literal, "1");
        assert_eq!(bound[1].literal, "2");
    }

    /// A text key is quoted, or the statement would not parse.
    #[test]
    fn a_text_key_is_quoted() {
        let rows = ResultSet::new(
            vec![Column::new("slug", "VARCHAR")],
            vec![vec![Cell::Text("o'brien".into())]],
        );
        let keys = vec!["slug".to_string()];
        let bound = key_bindings(&rows, 0, ("posts", &keys), Engine::Postgres).unwrap();
        assert_eq!(bound[0].literal, "'o''brien'");
    }
}
