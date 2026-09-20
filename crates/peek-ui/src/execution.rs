//! Running a query node and putting what comes back on the canvas.
//!
//! Ported from `~/labs/peek/src/canvas/executeQueries.ts`. The geometry and the document
//! mutations live in `peek_canvas::execution`; this is the async half — variable resolution, the
//! destructive-write gate, the round trip, and the `isRunning` flag.
//!
//! **The gate sits here, below the entry point, not on the Run button.** In the reference it
//! guards only `QueryNode`'s button, so the live-poll tick, the MCP tools and a multiplayer
//! joiner's remote execution all reach the database without it.

use gpui_kit::{App, Entity};
use peek_canvas::Document;
use peek_document::{NodeData, NodeId, QueryData};

use crate::database::Database;

/// What running a node should do, decided without touching gpui or the database.
///
/// Split out so every branch is unit-testable: the interesting cases are a destructive query
/// and an unresolved variable, and neither should need a live connection to prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Nothing to run: no such node, no query text, or a run already in flight.
    Refused,
    /// An unbounded `DELETE` or a `TRUNCATE` that has not been confirmed.
    NeedsConfirmation,
    /// A variable has no value. This becomes a `query-error` node **without** the database ever
    /// being asked, which is what stops `delete from t where id = @missing` running as
    /// `delete from t where id = @missing`.
    Undefined(Vec<String>),
    /// Run this SQL.
    Execute(String),
}

impl Plan {
    /// The message an [`Plan::Undefined`] shows, matching the reference's wording.
    fn undefined_message(names: &[String]) -> String {
        format!(
            "Undefined variables: {}",
            names
                .iter()
                .map(|name| format!("@{name}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Decides what running `node` means right now.
pub(crate) fn plan(document: &Document, node: &NodeId, confirmed: bool) -> Plan {
    let Some(data) = document
        .node(node)
        .and_then(|node| QueryData::get(&node.kind))
    else {
        return Plan::Refused;
    };
    if data.is_running == Some(true) || data.query.trim().is_empty() {
        return Plan::Refused;
    }
    if !confirmed && peek_lsp::is_unbounded_write(&data.query) {
        return Plan::NeedsConfirmation;
    }

    let variables = document.variables_for(node);
    let resolved = peek_lsp::substitute(&data.query, |name| variables.get(name).cloned());
    if resolved.missing.is_empty() {
        Plan::Execute(resolved.resolved)
    } else {
        Plan::Undefined(resolved.missing)
    }
}

/// What [`run`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Run {
    Started,
    /// The caller shows its "Run unbounded" affordance and calls again with `confirmed`.
    NeedsConfirmation,
    Refused,
}

/// Runs `node`'s query and places the result, or the error, beside it.
pub(crate) fn run(
    document: &Entity<Document>,
    node: &NodeId,
    confirmed: bool,
    cx: &mut App,
) -> Run {
    if !Database::is_connected(cx) {
        return Run::Refused;
    }
    let Some(session) = Database::session(cx) else {
        return Run::Refused;
    };

    let query = document
        .read(cx)
        .node(node)
        .and_then(|node| QueryData::get(&node.kind))
        .map(|data| data.query.clone())
        .unwrap_or_default();

    let sql = match plan(document.read(cx), node, confirmed) {
        Plan::Refused => return Run::Refused,
        Plan::NeedsConfirmation => return Run::NeedsConfirmation,
        Plan::Undefined(names) => {
            let message = Plan::undefined_message(&names);
            document.update(cx, |document, cx| {
                let (error, created) = document.place_query_error(node, &query, &message);
                document.focus_created(created.then_some(error).into_iter().collect());
                cx.notify();
            });
            return Run::Started;
        }
        Plan::Execute(sql) => sql,
    };

    set_running(document, node, true, cx);
    let document = document.clone();
    let node = node.clone();

    cx.spawn(async move |cx| {
        let outcome = session.query(sql).await;
        cx.update(|cx| {
            document.update(cx, |document, cx| {
                let (id, created) = match outcome {
                    Ok(Ok(rows)) => document.place_result(&node, (&query, 0), rows),
                    Ok(Err(error)) => document.place_query_error(&node, &query, &error.to_string()),
                    Err(_) => {
                        document.place_query_error(&node, &query, "the database runtime stopped")
                    }
                };
                document.focus_created(created.then_some(id).into_iter().collect());
                document.update_data::<QueryData>(&node, |data| data.is_running = Some(false));
                cx.notify();
            });
            // A run is what the reference labels on too, error or not: a statement worth
            // running is worth naming, and the name is of the query rather than its rows.
            crate::node::query::label::after_run(&document, &node, cx);
        });
    })
    .detach();

    Run::Started
}

/// Runs several statements from `source`, placing each result beside it.
///
/// `executeQueries`' fan-out, which the single-statement [`run`] does not cover: the follow-
/// references path issues one query per foreign key and wants them all on the canvas. Each is
/// executed and placed **in turn**, and each failure is caught on its own — statement three
/// failing does not stop statement four, exactly as the reference loops.
///
/// The source may be any node. Following a reference from a result makes that result the source,
/// so the new nodes stack under it and the edge says where they came from.
pub(crate) fn run_queries(
    document: &Entity<Document>,
    source: &NodeId,
    queries: Vec<String>,
    cx: &mut App,
) {
    if queries.is_empty() || !Database::is_connected(cx) {
        return;
    }
    let Some(session) = Database::session(cx) else {
        return;
    };
    let document = document.clone();
    let source = source.clone();

    cx.spawn(async move |cx| {
        let mut placed = Vec::new();
        for (index, query) in queries.into_iter().enumerate() {
            let outcome = session.query(query.clone()).await;
            let updated = cx.update(|cx| {
                document.update(cx, |document, cx| {
                    let (id, created) = match outcome {
                        Ok(Ok(rows)) => document.place_result(&source, (&query, index), rows),
                        Ok(Err(error)) => {
                            document.place_query_error(&source, &query, &error.to_string())
                        }
                        Err(_) => document.place_query_error(
                            &source,
                            &query,
                            "the database runtime stopped",
                        ),
                    };
                    cx.notify();
                    created.then_some(id)
                })
            });
            if let Some(id) = updated {
                placed.push(id);
            }
        }

        // `focusCreated` runs once the whole fan-out has landed, so the camera frames every
        // node the run placed rather than flying to each in turn.
        if !placed.is_empty() {
            cx.update(|cx| {
                document.update(cx, |document, cx| {
                    document.focus_created(placed);
                    cx.notify();
                });
            });
        }
    })
    .detach();
}

/// Re-runs the query that produced `result`, which is how a result refreshes after a row is
/// edited or deleted.
///
/// Going back through the query node rather than re-issuing the SQL means the variables are
/// re-resolved and the rows re-placed by the one path, exactly as a manual re-run would.
/// Already-confirmed: the statement being re-run is the one that produced these rows.
pub(crate) fn rerun_source(document: &Entity<Document>, result: &NodeId, cx: &mut App) {
    let Some(query) = document.read(cx).source_query_of(result) else {
        log::info!("peek: {result} has no query node to refresh from");
        return;
    };
    run(document, &query, true, cx);
}

/// `isRunning` is persisted, so a crash mid-query would leave a node spinning forever;
/// `peek_document::normalize` clears it on load for exactly that reason.
fn set_running(document: &Entity<Document>, node: &NodeId, running: bool, cx: &mut App) {
    document.update(cx, |document, cx| {
        document.update_data::<QueryData>(node, |data| data.is_running = Some(running));
        cx.notify();
    });
}

#[cfg(test)]
mod tests {
    use gpui_kit::AppContext;
    use peek_canvas::Document;
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{
        CanvasDocument, NodeData, NodeId, NodeType, QueryData, VariableData, VariableRow,
        VariableValue,
    };

    use super::{Plan, Run, plan, run, run_queries};

    fn with_query(sql: &str) -> (Document, NodeId) {
        let mut document = Document::load(CanvasDocument::empty());
        let id = document.create_node(
            NodeType::Query,
            Rect::new(Point::new(0.0, 0.0), Size::new(400.0, 300.0)),
        );
        document.update_data::<QueryData>(&id, |data| data.query = sql.to_string());
        (document, id)
    }

    #[test]
    fn a_plain_query_is_executed_as_written() {
        let (document, node) = with_query("select * from users");
        assert_eq!(
            plan(&document, &node, false),
            Plan::Execute("select * from users".to_string())
        );
    }

    #[test]
    fn an_empty_query_is_refused() {
        let (document, node) = with_query("   \n  ");
        assert_eq!(plan(&document, &node, false), Plan::Refused);
    }

    #[test]
    fn a_query_already_running_is_refused() {
        let (mut document, node) = with_query("select 1");
        document.update_data::<QueryData>(&node, |data| data.is_running = Some(true));
        assert_eq!(plan(&document, &node, false), Plan::Refused);
    }

    /// The two statements `isUnboundedWrite` flags. Without this gate they run on the first
    /// press of a button the user may only have meant to focus.
    #[test]
    fn an_unbounded_delete_needs_confirming_once() {
        let (document, node) = with_query("delete from users");
        assert_eq!(plan(&document, &node, false), Plan::NeedsConfirmation);
        assert_eq!(
            plan(&document, &node, true),
            Plan::Execute("delete from users".to_string()),
            "confirming lets the same query through"
        );
    }

    #[test]
    fn a_bounded_delete_runs_without_confirmation() {
        let (document, node) = with_query("delete from users where id = 1");
        assert!(matches!(plan(&document, &node, false), Plan::Execute(_)));
    }

    /// The gate is checked before substitution, so a `WHERE` that only exists once a variable
    /// resolves still counts as unbounded — which is the safe way round.
    #[test]
    fn the_gate_runs_before_variables_are_resolved() {
        let (document, node) = with_query("delete from users @clause");
        assert_eq!(plan(&document, &node, false), Plan::NeedsConfirmation);
    }

    #[test]
    fn an_undefined_variable_never_reaches_the_database() {
        let (document, node) = with_query("select * from t where id = @missing");
        assert_eq!(
            plan(&document, &node, false),
            Plan::Undefined(vec!["missing".to_string()])
        );
    }

    #[test]
    fn the_undefined_message_matches_the_reference() {
        let names = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            Plan::undefined_message(&names),
            "Undefined variables: @a, @b"
        );
    }

    #[test]
    fn a_connected_variable_node_resolves_its_references() {
        let (mut document, node) = with_query("select * from t limit @limit");
        let variable = document.create_node(
            NodeType::Variable,
            Rect::new(Point::default(), Size::new(220.0, 120.0)),
        );
        document.update_data::<VariableData>(&variable, |data| {
            data.rows = vec![VariableRow {
                name: "limit".to_string(),
                value: VariableValue::One("10".to_string()),
            }];
        });
        document.connect(&variable, &node);

        assert_eq!(
            plan(&document, &node, false),
            Plan::Execute("select * from t limit 10".to_string())
        );
    }

    /// A list variable becomes an `IN` list, with the list editor's trailing blank line dropped.
    #[test]
    fn a_list_variable_becomes_a_comma_separated_list() {
        let (mut document, node) = with_query("select * from t where id in (@ids)");
        let variable = document.create_node(
            NodeType::Variable,
            Rect::new(Point::default(), Size::new(220.0, 120.0)),
        );
        document.update_data::<VariableData>(&variable, |data| {
            data.rows = vec![VariableRow {
                name: "ids".to_string(),
                value: VariableValue::Many(vec!["1".to_string(), "2".to_string(), String::new()]),
            }];
        });
        document.connect(&variable, &node);

        assert_eq!(
            plan(&document, &node, false),
            Plan::Execute("select * from t where id in (1, 2)".to_string())
        );
    }

    #[test]
    fn a_node_that_is_not_a_query_is_refused() {
        let mut document = Document::load(CanvasDocument::empty());
        let text = document.create_node(
            NodeType::Text,
            Rect::new(Point::default(), Size::new(200.0, 100.0)),
        );
        assert_eq!(plan(&document, &text, false), Plan::Refused);
    }

    /// The whole path, against a real database: connect, run a query node, and find a result
    /// node beside it holding the rows.
    ///
    /// Opt-in on `PEEK_TEST_DATABASE_URL`, like `peek-db`'s live tests, so the default suite
    /// stays hermetic. This is the only test that exercises the gpui glue between
    /// [`crate::database::Database`] and [`peek_canvas::execution`]; everything below it is
    /// covered by the `plan` tests above.
    #[gpui_kit::test]
    async fn a_run_places_a_result_node_holding_its_rows(cx: &mut gpui_kit::TestAppContext) {
        let Ok(url) = std::env::var("PEEK_TEST_DATABASE_URL") else {
            return;
        };
        let connection = peek_config::DatabaseConnection {
            name: "test".to_string(),
            color: String::new(),
            url,
            ssh_tunnel: None,
        };

        // The driver runs on tokio's runtime and answers in real time, so the test executor
        // has to be allowed to actually sleep rather than advance a virtual clock.
        cx.background_executor.allow_parking();

        cx.update(|cx| {
            let config = peek_config::PeekConfig::default();
            crate::init(&config, cx);
            crate::database::Database::connect(&connection, cx);
        });

        // The driver runs on its own runtime, so parking the gpui executor is not enough.
        for _ in 0..100 {
            if cx.update(|cx| crate::database::Database::is_connected(cx)) {
                break;
            }
            cx.background_executor
                .timer(std::time::Duration::from_millis(50))
                .await;
        }
        assert!(
            cx.update(|cx| crate::database::Database::is_connected(cx)),
            "connected to the test database"
        );

        let document = cx.update(|cx| cx.new(|_| Document::load(CanvasDocument::empty())));
        let node = document.update(cx, |document, _| {
            let id = document.create_node(
                NodeType::Query,
                Rect::new(Point::new(0.0, 0.0), Size::new(400.0, 300.0)),
            );
            document.update_data::<QueryData>(&id, |data| {
                data.query = "select 1 as n, 'x' as label".to_string();
            });
            id
        });

        let started = cx.update(|cx| run(&document, &node, false, cx));
        assert_eq!(started, Run::Started);

        let result = NodeId::result_of(&node, 0);
        for _ in 0..100 {
            if document.read_with(cx, |document, _| document.node(&result).is_some()) {
                break;
            }
            cx.background_executor
                .timer(std::time::Duration::from_millis(50))
                .await;
        }

        document.read_with(cx, |document, _| {
            let placed = document.node(&result).expect("a result node was placed");
            assert!(
                placed.position.x > 0.0,
                "it sits to the right of its query, not on top of it"
            );
            let rows = document.result(&result).expect("its rows were stored");
            assert_eq!(rows.row_count(), 1);
            assert_eq!(rows.columns()[0].name, "n");
            assert_eq!(rows.columns()[1].name, "label");

            let query = document.node(&node).expect("the query node survived");
            let data = QueryData::get(&query.kind).unwrap();
            assert_eq!(
                data.is_running,
                Some(false),
                "the running flag is cleared, or the node would spin forever"
            );
        });
    }

    /// Following a reference places a result node holding the referenced rows.
    ///
    /// The half that needs a database: `run_queries` is the fan-out the single-statement `run`
    /// does not cover, and this is what a reference click ends in.
    ///
    /// Opt-in on `PEEK_TEST_DATABASE_URL`, like the rest of the live tests.
    #[gpui_kit::test]
    async fn following_a_reference_places_the_rows_it_points_at(cx: &mut gpui_kit::TestAppContext) {
        let Ok(url) = std::env::var("PEEK_TEST_DATABASE_URL") else {
            return;
        };
        cx.background_executor.allow_parking();

        cx.update(|cx| {
            let config = peek_config::PeekConfig::default();
            crate::init(&config, cx);
            crate::database::Database::connect(
                &peek_config::DatabaseConnection {
                    name: "test".to_string(),
                    color: String::new(),
                    url,
                    ssh_tunnel: None,
                },
                cx,
            );
        });
        for _ in 0..100 {
            if cx.update(|cx| crate::database::Database::is_connected(cx)) {
                break;
            }
            cx.background_executor
                .timer(std::time::Duration::from_millis(50))
                .await;
        }
        assert!(cx.update(|cx| crate::database::Database::is_connected(cx)));

        let document = cx.update(|cx| cx.new(|_| Document::load(CanvasDocument::empty())));
        // A result node stands in for the one a reference was clicked in.
        let source = document.update(cx, |document, _| {
            document.create_node(
                NodeType::Result,
                Rect::new(Point::new(0.0, 0.0), Size::new(600.0, 440.0)),
            )
        });

        cx.update(|cx| {
            run_queries(
                &document,
                &source,
                vec!["select 1 as id, 'referenced' as label".to_string()],
                cx,
            );
        });

        let placed = NodeId::result_of(&source, 0);
        for _ in 0..100 {
            if document.read_with(cx, |document, _| document.node(&placed).is_some()) {
                break;
            }
            cx.background_executor
                .timer(std::time::Duration::from_millis(50))
                .await;
        }

        document.read_with(cx, |document, _| {
            assert!(
                document.node(&placed).is_some(),
                "a result node was placed for the followed reference"
            );
            let rows = document.result(&placed).expect("holding its rows");
            assert_eq!(rows.row_count(), 1);
            assert_eq!(rows.columns()[1].name, "label");
            assert!(
                document
                    .edges()
                    .iter()
                    .any(|edge| edge.source == source && edge.target == placed),
                "and an edge back to where it came from"
            );
        });
    }
}
