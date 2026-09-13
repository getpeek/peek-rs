//! End-to-end checks against a real database.
//!
//! Opt-in: set `PEEK_TEST_DATABASE_URL` to a Postgres URL and these run; otherwise they skip, so
//! `cargo test --workspace` stays hermetic. Everything here is read-only — `select` only, no
//! temp tables, nothing that writes.

use peek_db::{Cell, Connection, Engine};

fn url() -> Option<String> {
    std::env::var("PEEK_TEST_DATABASE_URL").ok()
}

fn query(sql: &'static str) -> Option<Result<peek_db::ResultSet, peek_db::DbError>> {
    let url = url()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    Some(runtime.block_on(async move {
        let mut connection = Connection::open(&url).await.expect("connects");
        connection.query(sql).await
    }))
}

#[test]
fn scalars_decode_to_their_own_cell_kinds() {
    let Some(rows) = query(
        "select 1::int4 as i, 2.5::float8 as f, true as b, 'x'::text as t,
                null::int4 as n, '{\"a\":1}'::jsonb as j, '2020-01-02'::date as d",
    ) else {
        eprintln!("PEEK_TEST_DATABASE_URL unset; skipping");
        return;
    };
    let rows = rows.expect("query runs");
    assert_eq!(rows.row_count(), 1);
    let cell = |name: &str| rows.cell(0, rows.column_index(name).expect(name)).unwrap();

    assert_eq!(cell("i"), &Cell::Int(1));
    assert_eq!(cell("f"), &Cell::Float(2.5));
    assert_eq!(cell("b"), &Cell::Bool(true));
    assert_eq!(cell("t"), &Cell::Text("x".into()));
    assert_eq!(cell("n"), &Cell::Null, "a real NULL, not Undecodable");
    assert!(matches!(cell("j"), Cell::Json(_)));
    assert_eq!(cell("d"), &Cell::Text("2020-01-02".into()));
}

/// The reason `NUMERIC` rides as text: a double would round this off.
#[test]
fn numerics_keep_their_precision_as_text() {
    let Some(rows) = query("select 0.10000000000000000001::numeric as n") else {
        return;
    };
    assert_eq!(
        rows.expect("query runs").cell(0, 0),
        Some(&Cell::Text("0.10000000000000000001".into()))
    );
}

/// Types the reference has no arm for, which fell through its raw-bytes arm and became `null`.
#[test]
fn the_types_the_reference_dropped_now_decode() {
    let Some(rows) = query(
        "select array[1,2,3]::int4[] as arr, '\\xdeadbeef'::bytea as bin, '1 day'::interval as iv",
    ) else {
        return;
    };
    let rows = rows.expect("query runs");
    let cell = |name: &str| rows.cell(0, rows.column_index(name).expect(name)).unwrap();

    assert!(!cell("arr").is_null(), "an int array must not read as NULL");
    assert_eq!(
        cell("bin"),
        &Cell::Text("3q2+7w==".into()),
        "bytea is base64, as MySQL's blobs already were"
    );
    assert!(!cell("iv").is_null(), "an interval must not read as NULL");
}

#[test]
fn column_names_and_types_come_back_once_per_column() {
    let Some(rows) = query("select 1::int4 as id, 'a'::varchar as name") else {
        return;
    };
    let rows = rows.expect("query runs");
    let columns = rows.columns();
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name, "id");
    assert_eq!(columns[0].sql_type, "INT4");
    assert_eq!(columns[1].name, "name");
    assert_eq!(columns[1].sql_type, "VARCHAR");
}

#[test]
fn a_bad_statement_reports_the_servers_own_message() {
    let Some(outcome) = query("select * from a_table_that_does_not_exist") else {
        return;
    };
    let message = outcome.expect_err("fails").to_string();
    assert!(
        message.contains("does not exist"),
        "the driver's message reaches the user: {message}"
    );
}

#[test]
fn introspection_finds_tables_and_primary_keys() {
    let Some(url) = url() else { return };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let schema = runtime.block_on(async move {
        let mut connection = Connection::open(&url).await.expect("connects");
        connection.schema(&[]).await.expect("introspects")
    });
    assert!(!schema.tables.is_empty(), "the public schema has tables");
    assert!(
        schema.primary_keys.values().any(|keys| !keys.is_empty()),
        "at least one table has a primary key"
    );
}

#[test]
fn the_engine_comes_from_the_url_scheme() {
    let Some(url) = url() else { return };
    assert_eq!(Engine::from_url(&url), Engine::Postgres);
}
