//! The `MySQL` driver.
//!
//! Ported from `~/labs/peek/src-tauri/src/database/mysql.rs`, with the same NULL-versus-decode
//! -failure distinction the Postgres side makes.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sqlx::mysql::MySqlRow;
use sqlx::{Column as _, MySqlConnection, Row, TypeInfo, ValueRef};

use crate::error::DbError;
use peek_document::{Cell, Column, ResultSet};

pub(crate) fn result_set(rows: &[MySqlRow]) -> ResultSet {
    let Some(first) = rows.first() else {
        return ResultSet::default();
    };
    // `TINYINT(1)` is MySQL's boolean, and the width only survives in the full `to_string()`
    // spelling — `name()` says plain "TINYINT" for both.
    let widths: Vec<bool> = first
        .columns()
        .iter()
        .map(|column| column.type_info().to_string().contains("(1)"))
        .collect();
    let columns: Vec<Column> = first
        .columns()
        .iter()
        .map(|column| Column::new(column.name(), column.type_info().name()))
        .collect();
    let cells = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    cell(
                        row,
                        index,
                        &column.sql_type,
                        widths.get(index).copied().unwrap_or(false),
                    )
                })
                .collect()
        })
        .collect();
    ResultSet::new(columns, cells)
}

fn cell(row: &MySqlRow, index: usize, sql_type: &str, is_bool_width: bool) -> Cell {
    if row.try_get_raw(index).is_ok_and(|raw| raw.is_null()) {
        return Cell::Null;
    }
    match sql_type {
        "TINYINT" if is_bool_width => {
            decode::<bool>(row, index).map_or(Cell::Undecodable, Cell::Bool)
        }
        "TINYINT" => decode::<i8>(row, index).map_or(Cell::Undecodable, |v| Cell::Int(v.into())),
        "SMALLINT" => decode::<i16>(row, index).map_or(Cell::Undecodable, |v| Cell::Int(v.into())),
        "INT" | "MEDIUMINT" => {
            decode::<i32>(row, index).map_or(Cell::Undecodable, |v| Cell::Int(v.into()))
        }
        "BIGINT" => decode::<i64>(row, index).map_or(Cell::Undecodable, Cell::Int),
        "UNSIGNED TINYINT" => {
            decode::<u8>(row, index).map_or(Cell::Undecodable, |v| Cell::Int(v.into()))
        }
        "UNSIGNED SMALLINT" => {
            decode::<u16>(row, index).map_or(Cell::Undecodable, |v| Cell::Int(v.into()))
        }
        "UNSIGNED INT" | "UNSIGNED MEDIUMINT" => {
            decode::<u32>(row, index).map_or(Cell::Undecodable, |v| Cell::Int(v.into()))
        }
        // Values above i64::MAX keep their precision as text rather than wrapping.
        "UNSIGNED BIGINT" => decode::<u64>(row, index).map_or(Cell::Undecodable, |v| {
            i64::try_from(v).map_or_else(|_| Cell::Text(v.to_string()), Cell::Int)
        }),
        "FLOAT" => decode::<f32>(row, index).map_or(Cell::Undecodable, |v| Cell::Float(v.into())),
        "DOUBLE" => decode::<f64>(row, index).map_or(Cell::Undecodable, Cell::Float),
        "DECIMAL" | "NUMERIC" => decode::<rust_decimal::Decimal>(row, index)
            .map_or(Cell::Undecodable, |v| Cell::Text(v.to_string())),
        "DATE" => decode::<chrono::NaiveDate>(row, index).map_or(Cell::Undecodable, |v| {
            Cell::Text(v.format("%Y-%m-%d").to_string())
        }),
        "DATETIME" | "TIMESTAMP" => decode::<chrono::NaiveDateTime>(row, index)
            .map_or(Cell::Undecodable, |v| {
                Cell::Text(v.format("%Y-%m-%dT%H:%M:%S").to_string())
            }),
        "TIME" => decode::<chrono::NaiveTime>(row, index).map_or(Cell::Undecodable, |v| {
            Cell::Text(v.format("%H:%M:%S").to_string())
        }),
        "JSON" => decode::<serde_json::Value>(row, index).map_or(Cell::Undecodable, Cell::Json),
        "BINARY" | "VARBINARY" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" => {
            decode::<Vec<u8>>(row, index)
                .map_or(Cell::Undecodable, |bytes| Cell::Text(BASE64.encode(bytes)))
        }
        _ => decode::<String>(row, index).map_or(Cell::Undecodable, Cell::Text),
    }
}

fn decode<'r, T: sqlx::Decode<'r, sqlx::MySql> + sqlx::Type<sqlx::MySql>>(
    row: &'r MySqlRow,
    index: usize,
) -> Option<T> {
    row.try_get::<T, _>(index).ok()
}

pub(crate) async fn query(
    connection: &mut MySqlConnection,
    sql: &str,
) -> Result<ResultSet, DbError> {
    let rows = sqlx::query(sql)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| DbError::Query(error.to_string()))?;
    Ok(result_set(&rows))
}

pub(crate) async fn execute(connection: &mut MySqlConnection, sql: &str) -> Result<u64, DbError> {
    sqlx::query(sql)
        .execute(&mut *connection)
        .await
        .map(|done| done.rows_affected())
        .map_err(|error| DbError::Query(error.to_string()))
}

/// Introspection, scoped to the connected database.
///
/// `MySQL` reports `column_type` (`varchar(255)`) where Postgres reports a bare `udt_name`, so
/// its types carry their length. `information_schema` never lists temporary tables, which is
/// why `imported` is re-described with `SHOW COLUMNS`.
pub(crate) async fn schema(
    connection: &mut MySqlConnection,
    imported: &[String],
) -> Result<crate::Schema, DbError> {
    let mut schema = crate::Schema::default();

    let columns = sqlx::query(
        r"SELECT table_name, column_name, column_type
          FROM information_schema.columns
          WHERE table_schema = DATABASE()
          ORDER BY table_name, ordinal_position;",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| DbError::Schema {
        stage: "columns",
        message: error.to_string(),
    })?;
    for row in columns {
        schema
            .tables
            .entry(row.get::<String, _>(0))
            .or_default()
            .push((row.get::<String, _>(1), row.get::<String, _>(2)));
    }

    for table in imported {
        // A temp table that has since been dropped is not an error; skip it.
        let Ok(described) = sqlx::query(&format!(
            "SHOW COLUMNS FROM {}",
            crate::Engine::MySql.quote_identifier(table)
        ))
        .fetch_all(&mut *connection)
        .await
        else {
            continue;
        };
        let fields = described
            .iter()
            .map(|row| (row.get::<String, _>("Field"), row.get::<String, _>("Type")))
            .collect();
        schema.tables.insert(table.clone(), fields);
    }

    let foreign_keys = sqlx::query(
        r"SELECT TABLE_NAME AS referencing_table,
                 COLUMN_NAME AS referencing_column,
                 REFERENCED_TABLE_NAME AS referenced_table,
                 REFERENCED_COLUMN_NAME AS referenced_column
          FROM information_schema.KEY_COLUMN_USAGE
          WHERE TABLE_SCHEMA = DATABASE() AND REFERENCED_TABLE_NAME IS NOT NULL;",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| DbError::Schema {
        stage: "foreign keys",
        message: error.to_string(),
    })?;
    for row in foreign_keys {
        let referenced = format!(
            "{}.{}",
            row.get::<String, _>("referenced_table"),
            row.get::<String, _>("referenced_column")
        );
        let referencing = format!(
            "{}.{}",
            row.get::<String, _>("referencing_table"),
            row.get::<String, _>("referencing_column")
        );
        schema
            .references
            .entry(referenced)
            .or_default()
            .push(referencing);
    }

    let primary_keys = sqlx::query(
        r"SELECT tc.table_name, kcu.column_name
          FROM information_schema.table_constraints tc
          JOIN information_schema.key_column_usage kcu
            ON tc.constraint_name = kcu.constraint_name
           AND tc.table_schema = kcu.table_schema
           AND tc.table_name = kcu.table_name
          WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = DATABASE()
          ORDER BY tc.table_name, kcu.ordinal_position;",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|error| DbError::Schema {
        stage: "primary keys",
        message: error.to_string(),
    })?;
    for row in primary_keys {
        schema
            .primary_keys
            .entry(row.get::<String, _>("table_name"))
            .or_default()
            .push(row.get::<String, _>("column_name"));
    }

    Ok(schema)
}
