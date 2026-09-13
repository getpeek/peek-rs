//! The Postgres driver.
//!
//! Ported from `~/labs/peek/src-tauri/src/database/postgres.rs` with the type mapping widened:
//! the reference has arms for thirteen type names and lets everything else fall through a
//! raw-bytes-to-UTF-8 arm, which silently yields `null` for arrays, `bytea`, `interval` and
//! enums whenever sqlx negotiated the binary format. Its own `docs/database_drivers.md` calls
//! the `bytea` case "an inconsistency, not a rule".

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sqlx::postgres::PgRow;
use sqlx::{Column as _, PgConnection, Row, TypeInfo, ValueRef};

use crate::error::DbError;
use peek_document::{Cell, Column, ResultSet};

pub(crate) fn result_set(rows: &[PgRow]) -> ResultSet {
    let Some(first) = rows.first() else {
        return ResultSet::default();
    };
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
                .map(|(index, column)| cell(row, index, &column.sql_type))
                .collect()
        })
        .collect();
    ResultSet::new(columns, cells)
}

/// Decodes one cell.
///
/// Every arm distinguishes three outcomes the reference collapses into one: the column is NULL,
/// the column decoded, or the decoder failed. Only the last is [`Cell::Undecodable`].
fn cell(row: &PgRow, index: usize, sql_type: &str) -> Cell {
    if row.try_get_raw(index).is_ok_and(|raw| raw.is_null()) {
        return Cell::Null;
    }
    match sql_type {
        "BOOL" => decode::<bool>(row, index).map_or(Cell::Undecodable, Cell::Bool),
        "INT2" => decode::<i16>(row, index).map_or(Cell::Undecodable, |v| Cell::Int(v.into())),
        "INT4" => decode::<i32>(row, index).map_or(Cell::Undecodable, |v| Cell::Int(v.into())),
        "INT8" => decode::<i64>(row, index).map_or(Cell::Undecodable, Cell::Int),
        "FLOAT4" => decode::<f32>(row, index).map_or(Cell::Undecodable, |v| Cell::Float(v.into())),
        "FLOAT8" => decode::<f64>(row, index).map_or(Cell::Undecodable, Cell::Float),
        // Text, to keep the precision a double would round off. The Result node's aggregation
        // and the chart builder both parse it back.
        "NUMERIC" => decode::<rust_decimal::Decimal>(row, index)
            .map_or(Cell::Undecodable, |v| Cell::Text(v.to_string())),
        "TEXT" | "VARCHAR" | "CHAR" | "BPCHAR" | "NAME" | "CITEXT" => text(row, index),
        "UUID" => decode::<uuid::Uuid>(row, index)
            .map_or(Cell::Undecodable, |v| Cell::Text(v.to_string())),
        "DATE" => decode::<chrono::NaiveDate>(row, index).map_or(Cell::Undecodable, |v| {
            Cell::Text(v.format("%Y-%m-%d").to_string())
        }),
        "TIMESTAMP" => decode::<chrono::NaiveDateTime>(row, index).map_or(Cell::Undecodable, |v| {
            Cell::Text(v.format("%Y-%m-%dT%H:%M:%S").to_string())
        }),
        "TIMESTAMPTZ" => decode::<chrono::DateTime<chrono::Utc>>(row, index)
            .map_or(Cell::Undecodable, |v| Cell::Text(v.to_rfc3339())),
        "TIME" => decode::<chrono::NaiveTime>(row, index).map_or(Cell::Undecodable, |v| {
            Cell::Text(v.format("%H:%M:%S").to_string())
        }),
        "JSON" | "JSONB" => {
            decode::<serde_json::Value>(row, index).map_or(Cell::Undecodable, Cell::Json)
        }
        // Base64 rather than the reference's UTF-8 attempt, which turns most binary into `null`.
        "BYTEA" => decode::<Vec<u8>>(row, index)
            .map_or(Cell::Undecodable, |bytes| Cell::Text(BASE64.encode(bytes))),
        other if other.starts_with('_') => array(row, index),
        // Enums, ranges, inet, interval and anything else the server can render as text. These
        // arrive in the text format, so the string decoder reaches them.
        _ => text(row, index),
    }
}

fn decode<'r, T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>>(
    row: &'r PgRow,
    index: usize,
) -> Option<T> {
    row.try_get::<T, _>(index).ok()
}

/// A string, falling back to the raw bytes so a type sqlx has no decoder for still shows the
/// server's own text rendering rather than vanishing.
fn text(row: &PgRow, index: usize) -> Cell {
    if let Some(value) = decode::<String>(row, index) {
        return Cell::Text(value);
    }
    match row.try_get_raw(index).map(|raw| raw.as_bytes()) {
        Ok(Ok(bytes)) => std::str::from_utf8(bytes)
            .map_or(Cell::Undecodable, |text| Cell::Text(text.to_string())),
        _ => Cell::Undecodable,
    }
}

/// Postgres array types are named `_elem`. The reference has no arm for these at all; rendering
/// them as a JSON array is what lets the Result node show `{1,2,3}` as a value rather than NULL.
fn array(row: &PgRow, index: usize) -> Cell {
    if let Some(values) = decode::<Vec<String>>(row, index) {
        return Cell::Json(serde_json::Value::Array(
            values.into_iter().map(serde_json::Value::String).collect(),
        ));
    }
    if let Some(values) = decode::<Vec<i64>>(row, index) {
        return Cell::Json(serde_json::json!(values));
    }
    if let Some(values) = decode::<Vec<f64>>(row, index) {
        return Cell::Json(serde_json::json!(values));
    }
    text(row, index)
}

pub(crate) async fn query(connection: &mut PgConnection, sql: &str) -> Result<ResultSet, DbError> {
    let rows = sqlx::query(sql)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| DbError::Query(error.to_string()))?;
    Ok(result_set(&rows))
}

pub(crate) async fn execute(connection: &mut PgConnection, sql: &str) -> Result<u64, DbError> {
    sqlx::query(sql)
        .execute(&mut *connection)
        .await
        .map(|done| done.rows_affected())
        .map_err(|error| DbError::Query(error.to_string()))
}

/// The three introspection queries, kept verbatim from the reference except that each failure
/// now carries the driver's message instead of a fixed string.
///
/// The `pg_class` union is what makes temporary tables visible: `information_schema.columns`
/// never lists them, and imported CSV/JSON lands in a temp table.
pub(crate) async fn schema(connection: &mut PgConnection) -> Result<crate::Schema, DbError> {
    let mut schema = crate::Schema::default();

    let columns = sqlx::query(
        r"SELECT c.table_name, c.column_name, c.udt_name AS pg_type
          FROM information_schema.columns c
          WHERE c.table_schema = 'public'
          UNION ALL
          SELECT c.relname, a.attname, t.typname
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          JOIN pg_attribute a ON a.attrelid = c.oid
          JOIN pg_type t ON t.oid = a.atttypid
          WHERE c.relpersistence = 't' AND a.attnum > 0 AND NOT a.attisdropped;",
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

    let foreign_keys = sqlx::query(
        r"SELECT tc.table_name AS referencing_table,
                 kcu.column_name AS referencing_column,
                 ccu.table_name AS referenced_table,
                 ccu.column_name AS referenced_column
          FROM information_schema.table_constraints AS tc
          JOIN information_schema.key_column_usage AS kcu
            ON tc.constraint_name = kcu.constraint_name
           AND tc.table_schema = kcu.table_schema
          JOIN information_schema.constraint_column_usage AS ccu
            ON ccu.constraint_name = tc.constraint_name
           AND ccu.table_schema = tc.table_schema
          WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = 'public';",
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

    // ORDER BY ordinal_position is load-bearing: the composite-key DELETE builds its tuples in
    // this order, so a different one deletes the wrong rows.
    let primary_keys = sqlx::query(
        r"SELECT tc.table_name, kcu.column_name
          FROM information_schema.table_constraints tc
          JOIN information_schema.key_column_usage kcu
            ON tc.constraint_name = kcu.constraint_name
           AND tc.table_schema = kcu.table_schema
          WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = 'public'
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
