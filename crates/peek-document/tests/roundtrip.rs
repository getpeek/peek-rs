//! Round-trips a real workspace document (SQL text only; credentials live in settings.json).

use peek_document::{CanvasDocument, FALLBACK_SIZE, NodeType, normalize};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/plock-local.json");

fn strip_ephemeral(mut value: Value) -> Value {
    let pages = value["pages"].as_object_mut().unwrap();
    for page in pages.values_mut() {
        for node in page["nodes"].as_array_mut().unwrap() {
            let object = node.as_object_mut().unwrap();
            for key in [
                "selected",
                "dragging",
                "resizing",
                "measured",
                "className",
                "style",
            ] {
                object.remove(key);
            }
            // The TS app writes `isRunning: false`; we treat a stale `true` as a crash artefact
            // and clear it (below), so compare without it.
            if let Some(data) = object.get_mut("data").and_then(Value::as_object_mut) {
                data.remove("isRunning");
            }
        }
        for edge in page["edges"].as_array_mut().unwrap() {
            edge.as_object_mut().unwrap().remove("selected");
        }
        if page
            .get("regions")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            page.as_object_mut().unwrap().remove("regions");
        }
    }
    value
}

/// JS wrote `1`, serde writes `1.0`; both are the same JSON number.
fn normalize_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) => number.as_f64().map_or(Value::Null, |float| json!(float)),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize_numbers).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, item)| (key, normalize_numbers(item)))
                .collect(),
        ),
        other => other,
    }
}

fn first_difference(path: &str, left: &Value, right: &Value) -> Option<String> {
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            keys.into_iter()
                .find_map(|key| match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => first_difference(&format!("{path}.{key}"), x, y),
                    (x, y) => Some(format!("{path}.{key}: left={x:?} right={y:?}")),
                })
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(index, (x, y))| first_difference(&format!("{path}[{index}]"), x, y)),
        _ if left == right => None,
        _ => Some(format!("{path}: left={left} right={right}")),
    }
}

fn strip_is_running(mut value: Value) -> Value {
    for page in value["pages"].as_object_mut().unwrap().values_mut() {
        for node in page["nodes"].as_array_mut().unwrap() {
            if let Some(data) = node.get_mut("data").and_then(Value::as_object_mut) {
                data.remove("isRunning");
            }
        }
    }
    value
}

#[test]
fn fixture_has_no_credentials() {
    assert!(!FIXTURE.contains("postgres://"));
    assert!(!FIXTURE.contains("mysql://"));
}

#[test]
fn parses_the_real_document() {
    let document = CanvasDocument::from_json(FIXTURE).unwrap();
    assert_eq!(document.pages.len(), 4);
    assert_eq!(document.page_order.len(), 4);
    assert!(document.active_page().is_some());

    let nodes: usize = document.pages.values().map(|page| page.nodes.len()).sum();
    let edges: usize = document.pages.values().map(|page| page.edges.len()).sum();
    let regions: usize = document.pages.values().map(|page| page.regions.len()).sum();
    assert_eq!((nodes, edges, regions), (27, 19, 3));

    let kinds: std::collections::BTreeSet<_> = document
        .pages
        .values()
        .flat_map(|page| page.nodes.iter().filter_map(peek_document::Node::node_type))
        .collect();
    assert!(kinds.contains(&NodeType::Query));
    assert!(kinds.contains(&NodeType::Result));
    assert!(kinds.contains(&NodeType::Variable));
    assert!(kinds.contains(&NodeType::Barchart));

    for node in document.pages.values().flat_map(|page| &page.nodes) {
        let size = node.size();
        assert!(size.width > 0.0 && size.width != FALLBACK_SIZE || node.width.is_none());
    }
}

#[test]
fn clean_write_equals_original_minus_ephemeral_fields() {
    let document = CanvasDocument::from_json(FIXTURE).unwrap();
    let written: Value = serde_json::from_str(&document.to_json()).unwrap();
    let original: Value = serde_json::from_str(FIXTURE).unwrap();
    let written = normalize_numbers(strip_is_running(written));
    let original = normalize_numbers(strip_ephemeral(original));
    if let Some(difference) = first_difference("$", &written, &original) {
        panic!("clean write differs from original: {difference}");
    }
}

#[test]
fn round_trip_is_idempotent() {
    // `legacy_rows` is read-only, so compare the second write with the first rather than
    // the parsed structs.
    let first = CanvasDocument::from_json(FIXTURE).unwrap().to_json();
    let second = CanvasDocument::from_json(&first).unwrap().to_json();
    assert_eq!(first, second);
}

#[test]
fn tolerates_unknown_keys_and_kinds() {
    let raw = json!({
        "version": 1,
        "activePageId": "page_missing",
        "pageOrder": ["page_a"],
        "pages": {
            "page_a": {
                "id": "page_a", "name": "A", "future": true,
                "nodes": [
                    {"id": "x1", "type": "hologram", "position": {"x": 0, "y": 0}, "data": {"z": 1}},
                    {"id": "q1", "type": "query", "position": {"x": 0, "y": 0}, "extra": 3,
                     "data": {"query": "select 1", "isRunning": true, "novel": []}}
                ],
                "edges": [{"id": "a->b", "source": "a", "target": "b", "animated": true}],
                "viewport": {"x": 0, "y": 0, "zoom": 1}
            }
        },
        "trailer": "ignored"
    });
    let mut document = CanvasDocument::from_json(&raw.to_string()).unwrap();
    let notes = normalize(&mut document);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(document.active_page_id.as_str(), "page_a");
    let page = document.active_page().unwrap();
    assert_eq!(page.nodes.len(), 1);
    assert_eq!(page.nodes[0].node_type(), Some(NodeType::Query));
}

#[test]
fn rejects_other_versions() {
    assert!(
        CanvasDocument::from_json(r#"{"version":2,"activePageId":"p","pageOrder":[],"pages":{}}"#)
            .is_err()
    );
    assert!(CanvasDocument::from_json("{}").is_err());
}

mod agent_transcript {
    use peek_document::{AgentData, CanvasDocument, NodeData, NodeType};
    use serde_json::Value;

    const FIXTURE: &str = include_str!("fixtures/agent-transcript.json");

    fn transcript() -> AgentData {
        let document = CanvasDocument::from_json(FIXTURE).unwrap();
        let node = document
            .pages
            .values()
            .flat_map(|page| &page.nodes)
            .find(|node| node.node_type() == Some(NodeType::Agent))
            .unwrap();
        AgentData::get(&node.kind).unwrap().clone()
    }

    #[test]
    fn every_message_kind_parses() {
        let data = transcript();
        assert_eq!(data.query, "");
        assert_eq!(data.messages.len(), 12);

        let kinds: Vec<&str> = data.messages.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "system",
                "user",
                "context",
                "context",
                "thought",
                "acp_tool",
                "acp_tool",
                "plan",
                "tool_call",
                "tool_result",
                "assistant",
                "telepathy",
            ]
        );
    }

    #[test]
    fn a_legacy_schema_context_kind_is_kept_verbatim() {
        let data = transcript();
        assert_eq!(data.messages[2].context_kind.as_deref(), Some("schema"));
        assert_eq!(data.messages[3].context_kind.as_deref(), Some("result"));
    }

    #[test]
    fn a_failed_acp_tool_keeps_its_status_and_error_flag() {
        let data = transcript();
        let failed = &data.messages[6];
        assert_eq!(failed.tool_status.as_deref(), Some("failed"));
        assert_eq!(failed.is_error, Some(true));
        assert_eq!(failed.tool_kind.as_deref(), Some("execute"));
        // `toolName` carries the ACP *title*, not an identifier.
        assert_eq!(failed.tool_name.as_deref(), Some("Run shell command"));
    }

    #[test]
    fn a_plan_keeps_its_entries_in_order() {
        let data = transcript();
        let entries = data.messages[7].plan_entries.as_ref().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, "completed");
        assert_eq!(entries[1].priority, "medium");
    }

    #[test]
    fn a_tool_call_keeps_its_arbitrary_arguments() {
        let data = transcript();
        let calls = data.messages[8].tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].name, "create_query_node");
        assert_eq!(calls[0].args["query"], "select 1");
        assert_eq!(data.messages[9].tool_call_id.as_deref(), Some("call_1"));
    }

    /// The on-disk format is frozen: writing the parsed transcript back must reproduce every
    /// key, including the unknown `telepathy` kind and the legacy `contextKind`.
    #[test]
    fn the_transcript_round_trips_key_for_key() {
        let source: Value = serde_json::from_str(FIXTURE).unwrap();
        let expected = &source["pages"]["page_agent001"]["nodes"][0]["data"]["messages"];

        let written = serde_json::to_value(transcript().messages).unwrap();
        assert_eq!(&written, expected);
    }

    /// `provider` is an `Option` that must vanish rather than serialize as null — the
    /// TypeScript app omits it on a fresh node.
    #[test]
    fn an_unset_provider_is_omitted_not_null() {
        let data = AgentData::default();
        let json = serde_json::to_string(&data).unwrap();
        assert_eq!(json, r#"{"query":"","messages":[]}"#);
    }
}
