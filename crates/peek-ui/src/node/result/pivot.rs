//! Toggling a result between the table and the record view: `togglePivot.ts`.
//!
//! Pivot is a narrow two-column layout, so it wants a slimmer, taller node than the wide table.
//! Pivoting reshapes the node and remembers the size it had; toggling back restores it.

use peek_canvas::Document;
use peek_document::geometry::Size;
use peek_document::{NodeData, NodeId, ResultData};

const PIVOT_WIDTH: f64 = 460.0;
const PIVOT_CHROME_HEIGHT: f64 = 130.0;
const PIVOT_ROW_HEIGHT: f64 = 34.0;
const PIVOT_MIN_HEIGHT: f64 = 260.0;
const PIVOT_MAX_HEIGHT: f64 = 900.0;
/// What the reference falls back to for a node carrying no explicit size.
const DEFAULT_WIDTH: f64 = 620.0;
const DEFAULT_HEIGHT: f64 = 640.0;

/// Height grows with the column count, one field per row, clamped at both ends.
fn pivot_height(column_count: usize) -> f64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a result's column count is far below 2^53"
    )]
    let fit = PIVOT_CHROME_HEIGHT + column_count as f64 * PIVOT_ROW_HEIGHT;
    fit.clamp(PIVOT_MIN_HEIGHT, PIVOT_MAX_HEIGHT)
}

/// Flips one result node between the table and the record view, as one undo step.
///
/// Returns whether anything changed, so a caller acting on a multi-node selection can tell an
/// empty command from one that did work.
pub(crate) fn toggle(document: &mut Document, id: &NodeId) -> bool {
    let Some(node) = document.node(id) else {
        return false;
    };
    let Some(data) = ResultData::get(&node.kind) else {
        return false;
    };
    let pivoted = data.pivoted.unwrap_or_default();
    let restore = data.pre_pivot_size;
    let size = node.size();
    let columns = document.result(id).map_or(0, |rows| rows.columns().len());

    document.transaction(|document| {
        if pivoted {
            document.update_data::<ResultData>(id, |data| {
                data.pivoted = None;
                data.pre_pivot_size = None;
            });
            let restored = restore.unwrap_or(Size::new(DEFAULT_WIDTH, DEFAULT_HEIGHT));
            document.set_size(id, restored);
            return;
        }
        document.update_data::<ResultData>(id, |data| {
            data.pivoted = Some(true);
            data.pre_pivot_size = Some(size);
        });
        document.set_size(
            id,
            Size::new(size.width.min(PIVOT_WIDTH), pivot_height(columns)),
        );
    });
    document.checkpoint();
    true
}

#[cfg(test)]
mod tests {
    use peek_canvas::Document;
    use peek_document::geometry::Size;
    use peek_document::{CanvasDocument, Cell, Column, NodeData, NodeId, ResultData, ResultSet};

    use super::{PIVOT_MIN_HEIGHT, PIVOT_WIDTH, toggle};

    const DOCUMENT: &str = r#"{
      "version": 1,
      "pageOrder": ["page_1"],
      "activePageId": "page_1",
      "pages": {
        "page_1": {
          "id": "page_1",
          "name": "one",
          "nodes": [{
            "id": "query_aaaaaaaa-result-0",
            "type": "result",
            "position": { "x": 0, "y": 0 },
            "width": 620,
            "height": 640,
            "data": { "query": "select * from users" }
          }],
          "edges": [],
          "viewport": { "x": 0, "y": 0, "zoom": 1 }
        }
      }
    }"#;

    fn node() -> NodeId {
        NodeId::from("query_aaaaaaaa-result-0")
    }

    fn document() -> Document {
        let mut document = Document::load(CanvasDocument::from_json(DOCUMENT).unwrap());
        document.set_result(
            node(),
            ResultSet::new(
                vec![Column::new("id", "INT4"), Column::new("name", "VARCHAR")],
                vec![vec![Cell::Int(1), Cell::Text("one".to_string())]],
            ),
        );
        document
    }

    /// The result node's `data` object exactly as it lands on disk.
    fn written_data(document: &Document) -> serde_json::Value {
        let json: serde_json::Value = serde_json::from_str(&document.inner().to_json()).unwrap();
        json["pages"]["page_1"]["nodes"][0]["data"].clone()
    }

    fn data(document: &Document) -> ResultData {
        ResultData::get(&document.node(&node()).unwrap().kind)
            .unwrap()
            .clone()
    }

    #[test]
    fn pivoting_narrows_the_node_and_remembers_its_size() {
        let mut document = document();
        assert!(toggle(&mut document, &node()));

        let data = data(&document);
        assert_eq!(data.pivoted, Some(true));
        assert_eq!(data.pre_pivot_size, Some(Size::new(620.0, 640.0)));
        assert_eq!(
            document.node(&node()).unwrap().size(),
            Size::new(PIVOT_WIDTH, PIVOT_MIN_HEIGHT)
        );
    }

    #[test]
    fn toggling_back_restores_the_size_and_writes_neither_field() {
        let mut document = document();
        toggle(&mut document, &node());
        toggle(&mut document, &node());

        let data = data(&document);
        assert_eq!(data.pivoted, None, "so a clean write drops the field");
        assert_eq!(data.pre_pivot_size, None);
        assert_eq!(
            document.node(&node()).unwrap().size(),
            Size::new(620.0, 640.0)
        );
    }

    /// One press is one step: the data edit and the resize must undo together.
    #[test]
    fn a_pivot_is_a_single_undo_step() {
        let mut document = document();
        toggle(&mut document, &node());
        assert!(document.undo());

        let data = data(&document);
        assert_eq!(data.pivoted, None);
        assert_eq!(
            document.node(&node()).unwrap().size(),
            Size::new(620.0, 640.0)
        );
    }

    /// The workspace documents are shared on disk with the TypeScript app, whose `ResultData`
    /// declares `pivoted?: boolean` and `prePivotSize?: { width: number; height: number }`
    /// (`~/labs/peek/src/canvas/types.ts:27`). This is the one thing in the pivot that cannot be
    /// fixed later, so it is asserted against the written JSON rather than the in-memory model.
    #[test]
    fn the_pivot_fields_are_written_in_the_shape_the_reference_reads() {
        let mut document = document();
        toggle(&mut document, &node());

        let written = written_data(&document);
        assert_eq!(written["pivoted"], serde_json::json!(true));
        assert_eq!(
            written["prePivotSize"],
            serde_json::json!({ "width": 620.0, "height": 640.0 })
        );

        // And a document written by this app reopens here with the flip intact.
        let reopened =
            Document::load(CanvasDocument::from_json(&document.inner().to_json()).unwrap());
        assert_eq!(data(&reopened).pivoted, Some(true));
        assert_eq!(
            data(&reopened).pre_pivot_size,
            Some(Size::new(620.0, 640.0))
        );
    }

    /// Toggling off drops both keys rather than writing `pivoted: false`. The reference reads
    /// `data.pivoted ?? false`, so an absent key and a false one mean the same thing there, and
    /// absent is what a clean write leaves behind.
    #[test]
    fn toggling_back_writes_neither_key() {
        let mut document = document();
        toggle(&mut document, &node());
        toggle(&mut document, &node());

        let written = written_data(&document);
        assert!(written.get("pivoted").is_none(), "{written}");
        assert!(written.get("prePivotSize").is_none(), "{written}");
        assert_eq!(written["query"], serde_json::json!("select * from users"));
    }

    #[test]
    fn a_node_that_is_not_a_result_is_left_alone() {
        let mut document = document();
        assert!(!toggle(&mut document, &NodeId::from("text_bbbbbbbb")));
    }
}
