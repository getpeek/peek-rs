//! Feeding the local model the rows the user actually ran.
//!
//! The agent cannot run a query itself, so a Result node wired into it is how data reaches the
//! model at all. `~/labs/peek/src/canvas/nodes/Agent/useAgentContextSync.ts` inlines the source
//! query and its rows as a `context` message, deduplicated by a hash of the rows, so re-running
//! a query produces a fresh one and the reader sees "Context updated".
//!
//! This is the Ollama path only. An ACP agent reads the canvas through Peek's MCP server, which
//! is both richer and does not spend the context window on rows it may not need.
//!
//! A Query node wired into the agent is sent as a reference instead — its id, SQL and whether it
//! is live polling — so the user can say "this node" and the model knows which one they mean.
//! Unlike rows, both backends get it: an ACP agent can read the canvas, but not which card the
//! user has in mind. The reference has no query -> agent edge, so this has no counterpart there;
//! its `context_kind` is `"query"`, a free string the TypeScript app renders as any context.
//!
//! Two deliberate differences from the reference. It syncs on every render, which writes to the
//! document from a render pass; this gathers at the start of a turn instead, which is the moment
//! the rows are actually needed. And it inlines every row without a bound — a million-row result
//! would be pasted whole into the prompt — so this caps them and says how many were left out.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use peek_canvas::Document;
use peek_document::{
    AgentMessage, LiveInterval, NodeId, NodeType, QueryData, ResultData, ResultSet,
};

use super::view::now_ms;

/// How many rows of a result reach the model. Enough to characterise a shape; not enough to
/// bury the question.
const MAX_ROWS: usize = 50;

/// The `context` messages a turn should stage, given what is already in the transcript.
///
/// Returns only what is new: a result the model has already been shown, unchanged, is skipped,
/// which is what stops every turn from re-pasting the same table.
pub(super) fn gather(
    document: &Document,
    agent: &NodeId,
    seen: &[AgentMessage],
) -> Vec<AgentMessage> {
    let mut candidates = query_references(document, agent);
    candidates.extend(results(document, agent));
    unseen(candidates, seen)
}

/// One `context` message per Query node wired into `agent`, whether or not it has been sent.
pub(super) fn query_references(document: &Document, agent: &NodeId) -> Vec<AgentMessage> {
    sources(document, agent, NodeType::Query)
        .into_iter()
        .filter_map(|source| {
            let data: &QueryData = peek_document::NodeData::get(&document.node(&source)?.kind)?;
            let live = match data.live_interval_ms {
                Some(LiveInterval::EveryMs(ms)) => Some(ms),
                Some(LiveInterval::Off) | None => None,
            };
            let mut message =
                AgentMessage::new("context", reference(&source, data, live), now_ms());
            message.context_key = Some(query_fingerprint(&source, data, live));
            message.context_kind = Some(QUERY_CONTEXT.to_string());
            Some(message)
        })
        .collect()
}

/// The `context_kind` of a query reference.
pub(super) const QUERY_CONTEXT: &str = "query";

/// The messages in `candidates` whose key is not already in the transcript.
pub(super) fn unseen(candidates: Vec<AgentMessage>, seen: &[AgentMessage]) -> Vec<AgentMessage> {
    let known: Vec<&str> = seen
        .iter()
        .filter(|message| message.is("context"))
        .filter_map(|message| message.context_key.as_deref())
        .collect();
    candidates
        .into_iter()
        .filter(|message| {
            message
                .context_key
                .as_deref()
                .is_none_or(|key| !known.contains(&key))
        })
        .collect()
}

fn results(document: &Document, agent: &NodeId) -> Vec<AgentMessage> {
    sources(document, agent, NodeType::Result)
        .into_iter()
        .filter_map(|source| {
            let rows = document.result(&source)?;
            if rows.rows().is_empty() {
                return None;
            }
            let key = fingerprint(&source, rows);
            // The SQL that produced these rows is recorded on the result itself by
            // `place_result`, so it is right even if the query node has since been edited.
            let query = document
                .node(&source)
                .and_then(|node| peek_document::NodeData::get(&node.kind))
                .map_or(String::new(), |data: &ResultData| data.query.clone());

            let mut message = AgentMessage::new("context", render(&query, rows), now_ms());
            message.context_key = Some(key);
            message.context_kind = Some("result".to_string());
            Some(message)
        })
        .collect()
}

/// Nodes of `kind` feeding this agent, in edge order so two of them resolve the same way every
/// run.
fn sources(document: &Document, agent: &NodeId, kind: NodeType) -> Vec<NodeId> {
    let mut edges: Vec<&peek_document::Edge> = document
        .edges()
        .iter()
        .filter(|edge| &edge.target == agent)
        .collect();
    edges.sort_by(|a, b| a.id.cmp(&b.id));

    edges
        .into_iter()
        .map(|edge| edge.source.clone())
        .filter(|source| {
            document
                .node(source)
                .and_then(peek_document::Node::node_type)
                == Some(kind)
        })
        .collect()
}

/// What the model is told about a wired query. The id is the handle the canvas tools take, so
/// "this node" resolves to something the agent can act on.
fn reference(id: &NodeId, data: &QueryData, live: Option<u64>) -> String {
    let polling = live.map_or("It is not live polling.".to_string(), |ms| {
        format!("It is live polling every {ms} ms.")
    });
    format!(
        "The user wired Query node `{id}` into this conversation. When they say \"this node\" \
         or \"this query\", they mean it. {polling}\n\nSQL:\n{}\n",
        data.query
    )
}

/// The query and its rows, as the reference lays them out.
fn render(query: &str, rows: &ResultSet) -> String {
    let headers: Vec<&str> = rows
        .columns()
        .iter()
        .map(|column| column.name.as_str())
        .collect();
    let shown: Vec<String> = rows
        .rows()
        .iter()
        .take(MAX_ROWS)
        .map(|row| {
            row.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(";")
        })
        .collect();

    let mut body = format!(
        "\nQuery: {query}\n\nresult:\n{}\n{}\n",
        headers.join(";"),
        shown.join("\n")
    );
    if rows.rows().len() > MAX_ROWS {
        use std::fmt::Write as _;
        let hidden = rows.rows().len() - MAX_ROWS;
        let _ = write!(body, "\n({hidden} more rows not shown)\n");
    }
    body
}

/// A query reference changes, and is sent again, when its SQL or its polling does.
fn query_fingerprint(id: &NodeId, data: &QueryData, live: Option<u64>) -> String {
    let mut hasher = DefaultHasher::new();
    id.as_str().hash(&mut hasher);
    data.query.hash(&mut hasher);
    live.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Identifies a result by what it contains, so re-running the same query with the same answer
/// is the same context and a changed answer is a new one.
///
/// Not the reference's sha1 — this is an opaque dedupe token that nothing compares across
/// documents, and a cryptographic hash would be a dependency for no gain. A transcript written
/// here and reopened in the TypeScript app may therefore insert one extra context message.
fn fingerprint(source: &NodeId, rows: &ResultSet) -> String {
    let mut hasher = DefaultHasher::new();
    source.as_str().hash(&mut hasher);
    for column in rows.columns() {
        column.name.hash(&mut hasher);
    }
    for row in rows.rows() {
        for cell in row {
            cell.to_string().hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::{MAX_ROWS, gather};
    use peek_canvas::Document;
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{
        AgentData, AgentMessage, CanvasDocument, Cell, Column, LiveInterval, Node, NodeId,
        NodeKind, NodeType, QueryData, ResultData, ResultSet,
    };

    fn canvas(row_count: usize) -> Document {
        let mut persisted = CanvasDocument::empty();
        let page = persisted
            .pages
            .get_mut(&persisted.active_page_id)
            .expect("one page");

        let mut push = |id: &str, kind: NodeKind, node_type: NodeType| {
            let mut node = Node::new(
                node_type,
                Rect::new(Point::new(0.0, 0.0), Size::new(300.0, 200.0)),
            );
            node.id = NodeId::from(id);
            node.kind = kind;
            page.nodes.push(node);
        };
        push(
            "agent_1",
            NodeKind::Agent(AgentData::default()),
            NodeType::Agent,
        );
        push(
            "query_1",
            NodeKind::Query(QueryData {
                query: "select id from users".to_string(),
                ..QueryData::default()
            }),
            NodeType::Query,
        );
        push(
            "query_1-result-0",
            NodeKind::Result(ResultData {
                query: "select id from users".to_string(),
                ..ResultData::default()
            }),
            NodeType::Result,
        );

        let mut document = Document::load(persisted);
        document.connect(&NodeId::from("query_1-result-0"), &NodeId::from("agent_1"));
        let rows: Vec<Vec<Cell>> = (0..row_count)
            .map(|index| vec![Cell::Text(index.to_string())])
            .collect();
        document.set_result(
            NodeId::from("query_1-result-0"),
            ResultSet::new(
                vec![Column {
                    name: "id".to_string(),
                    sql_type: "int4".to_string(),
                }],
                rows,
            ),
        );
        document
    }

    fn gathered(document: &Document, seen: &[AgentMessage]) -> Vec<AgentMessage> {
        gather(document, &NodeId::from("agent_1"), seen)
    }

    #[test]
    fn a_wired_result_becomes_context_carrying_its_query_and_rows() {
        let document = canvas(2);
        let context = gathered(&document, &[]);

        assert_eq!(context.len(), 1);
        assert_eq!(context[0].kind, "context");
        assert!(context[0].message.contains("select id from users"));
        assert!(context[0].message.contains("id"));
        assert!(context[0].context_key.is_some());
    }

    /// Otherwise every turn re-pastes the same table and the question drowns in it.
    #[test]
    fn a_result_the_model_has_already_seen_is_not_sent_again() {
        let document = canvas(2);
        let first = gathered(&document, &[]);
        assert_eq!(first.len(), 1);

        let again = gathered(&document, &first);
        assert!(again.is_empty());
    }

    #[test]
    fn a_result_that_changed_is_sent_again() {
        let document = canvas(2);
        let seen = gathered(&document, &[]);

        let rerun = canvas(3);
        let after = gathered(&rerun, &seen);
        assert_eq!(after.len(), 1, "different rows are a different context");
    }

    /// The reference inlines every row; a large result would be pasted whole into the prompt.
    #[test]
    fn a_large_result_is_capped_and_says_how_much_was_left_out() {
        let document = canvas(MAX_ROWS + 7);
        let context = gathered(&document, &[]);

        let lines = context[0].message.lines().count();
        assert!(lines < MAX_ROWS + 12, "the rows were capped, got {lines}");
        assert!(context[0].message.contains("7 more rows not shown"));
    }

    fn wired_query(live: Option<u64>) -> Document {
        let mut document = canvas(0);
        let query = NodeId::from("query_1");
        document.connect(&query, &NodeId::from("agent_1"));
        document.update_data(&query, |data: &mut QueryData| {
            data.live_interval_ms = live.map(LiveInterval::EveryMs);
        });
        document
    }

    #[test]
    fn a_wired_query_is_referenced_by_id_sql_and_polling() {
        let context = gathered(&wired_query(None), &[]);

        assert_eq!(context.len(), 1);
        assert_eq!(context[0].context_kind.as_deref(), Some("query"));
        let text = &context[0].message;
        assert!(text.contains("`query_1`"), "{text}");
        assert!(text.contains("this node"), "{text}");
        assert!(text.contains("select id from users"), "{text}");
        assert!(text.contains("not live polling"), "{text}");

        let live = gathered(&wired_query(Some(5000)), &[]);
        assert!(live[0].message.contains("live polling every 5000 ms"));
    }

    #[test]
    fn a_query_reference_is_sent_again_only_when_its_polling_or_sql_changes() {
        let seen = gathered(&wired_query(None), &[]);
        assert!(gathered(&wired_query(None), &seen).is_empty());
        assert_eq!(gathered(&wired_query(Some(5000)), &seen).len(), 1);
    }

    #[test]
    fn a_result_with_no_rows_is_not_worth_sending() {
        let document = canvas(0);
        assert!(gathered(&document, &[]).is_empty());
    }
}
