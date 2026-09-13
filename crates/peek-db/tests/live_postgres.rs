//! End-to-end checks against a real database.
//!
//! Opt-in: set `PEEK_TEST_DATABASE_URL` to a Postgres URL and these run; otherwise they skip, so
//! `cargo test --workspace` stays hermetic.
//!
//! Nothing here can touch real data. The read tests are `select` only, and the write tests work
//! exclusively in a `CREATE TEMP TABLE`, which lives on the connection and disappears with it —
//! no schema qualification, no cleanup to forget.

use peek_db::mutation;
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

/// The statements the Result node's inline editing runs, end to end: build, execute, read back.
///
/// In a temp table, so a mistake here cannot reach anything real.
#[test]
fn an_update_built_from_a_primary_key_changes_exactly_one_row() {
    let Some(url) = url() else {
        eprintln!("PEEK_TEST_DATABASE_URL unset; skipping");
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async move {
        let mut connection = Connection::open(&url).await.expect("connects");
        connection
            .execute("create temp table peek_edit_probe (id int primary key, name text)")
            .await
            .expect("temp table");
        connection
            .execute("insert into peek_edit_probe values (1, 'one'), (2, 'two')")
            .await
            .expect("seed");

        // Exactly what the node builds: one column, pinned by its primary key.
        let statement = mutation::build_update(
            Engine::Postgres,
            ("peek_edit_probe", "name"),
            &mutation::format_literal("edited", "TEXT", Engine::Postgres),
            &[mutation::KeyBinding {
                column: "id".to_string(),
                literal: "1".to_string(),
            }],
        )
        .expect("builds");
        let changed = connection.execute(&statement).await.expect("runs");
        assert_eq!(changed, 1, "one row, not the table");

        let rows = connection
            .query("select id, name from peek_edit_probe order by id")
            .await
            .expect("reads back");
        assert_eq!(rows.cell(0, 1), Some(&Cell::Text("edited".into())));
        assert_eq!(
            rows.cell(1, 1),
            Some(&Cell::Text("two".into())),
            "the other row was untouched"
        );

        // And the delete the same way.
        let statement = mutation::build_delete(
            Engine::Postgres,
            "peek_edit_probe",
            &["id".to_string()],
            &[vec![mutation::KeyBinding {
                column: "id".to_string(),
                literal: "2".to_string(),
            }]],
        )
        .expect("builds");
        let removed = connection.execute(&statement).await.expect("runs");
        assert_eq!(removed, 1);

        let rows = connection
            .query("select id from peek_edit_probe")
            .await
            .expect("reads back");
        assert_eq!(rows.row_count(), 1, "only the named row went");
    });
}

/// A value with a quote in it must survive the round trip rather than break the statement.
#[test]
fn a_quoted_value_round_trips_through_an_update() {
    let Some(url) = url() else { return };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    runtime.block_on(async move {
        let mut connection = Connection::open(&url).await.expect("connects");
        connection
            .execute("create temp table peek_quote_probe (id int primary key, name text)")
            .await
            .expect("temp table");
        connection
            .execute("insert into peek_quote_probe values (1, 'plain')")
            .await
            .expect("seed");

        let statement = mutation::build_update(
            Engine::Postgres,
            ("peek_quote_probe", "name"),
            &mutation::format_literal("o'brien; drop table x", "TEXT", Engine::Postgres),
            &[mutation::KeyBinding {
                column: "id".to_string(),
                literal: "1".to_string(),
            }],
        )
        .expect("builds");
        connection.execute(&statement).await.expect("runs");

        let rows = connection
            .query("select name from peek_quote_probe")
            .await
            .expect("reads back");
        assert_eq!(
            rows.cell(0, 0),
            Some(&Cell::Text("o'brien; drop table x".into())),
            "stored verbatim, not executed"
        );
    });
}
