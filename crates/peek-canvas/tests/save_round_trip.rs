//! The guarantee that editing a real document does not corrupt it: a full create / move /
//! delete / undo cycle against the 27-node fixture must serialize back to exactly what was
//! loaded, minus the ephemeral fields a clean write always drops.

use peek_canvas::Document;
use peek_canvas::{Point, Rect, Size};
use peek_document::{CanvasDocument, NodeType, TextData, normalize};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("../../peek-document/tests/fixtures/plock-local.json");

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

fn loaded() -> Document {
    let mut document = CanvasDocument::from_json(FIXTURE).unwrap();
    normalize(&mut document);
    Document::load(document)
}

fn expected() -> Value {
    let original: Value = serde_json::from_str(FIXTURE).unwrap();
    normalize_numbers(strip_ephemeral(original))
}

fn written(document: &Document) -> Value {
    normalize_numbers(serde_json::from_str(&document.inner().to_json()).unwrap())
}

fn rect(x: f64, y: f64) -> Rect {
    Rect::new(Point::new(x, y), Size::new(200.0, 100.0))
}

#[test]
fn an_untouched_document_writes_back_unchanged() {
    let document = loaded();
    assert_eq!(
        first_difference("$", &expected(), &written(&document)),
        None
    );
}

#[test]
fn edit_then_undo_serializes_back_to_the_fixture() {
    let mut document = loaded();
    let nodes_before = document.nodes().len();

    let created = document.create_node(NodeType::Text, rect(1_000.0, 1_000.0));
    document.checkpoint();
    assert!(document.update_data::<TextData>(&created, |data| {
        data.text = "scratch".to_string();
    }));
    document.checkpoint();
    document.translate_nodes(std::slice::from_ref(&created), Point::new(25.0, -40.0));
    document.checkpoint();

    let existing = document.nodes()[0].id.clone();
    document.translate_nodes(std::slice::from_ref(&existing), Point::new(13.0, 7.0));
    document.checkpoint();
    document.remove_nodes(&[existing]);
    document.checkpoint();

    assert_ne!(
        first_difference("$", &expected(), &written(&document)),
        None,
        "the edits really did change the document"
    );

    while document.undo() {}

    assert_eq!(document.nodes().len(), nodes_before);
    assert_eq!(
        first_difference("$", &expected(), &written(&document)),
        None,
        "every untouched node came back byte-identical"
    );
}

#[test]
fn redo_replays_the_whole_edit() {
    let mut document = loaded();
    let created = document.create_node(NodeType::Text, rect(1_000.0, 1_000.0));
    document.checkpoint();
    let after_create = written(&document);

    assert!(document.undo());
    assert!(document.node(&created).is_none());

    assert!(document.redo());
    assert_eq!(
        first_difference("$", &after_create, &written(&document)),
        None
    );
}
