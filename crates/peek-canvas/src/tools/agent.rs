//! The agent node's adapter over the same tool surface.
//!
//! The agent schema is terser than MCP's: it drops `pageId` (a local model has no business
//! choosing a page) and makes `position`, `size`, `height` and `order` optional, because a small
//! model places nodes badly. This fills in what it left out, so the executor itself sees the
//! same complete arguments the MCP bridge sends and needs no second code path.
//!
//! `~/labs/peek/src/canvas/nodes/Agent/useAgentTools.ts` does exactly this before delegating to
//! the shared mutations.

use peek_document::geometry::Point;
use peek_document::{NodeId, NodeType};
use serde_json::Value;

use super::ToolCall;
use crate::model::Document;

/// The gap a placed node leaves to the right of the agent that asked for it, and the vertical
/// pitch between successive ones.
const GUTTER: f64 = 80.0;
const ROW_GAP: f64 = 40.0;

/// Fills in the arguments the agent schema lets the model omit.
#[must_use]
pub fn agent_params(document: &Document, agent: &NodeId, call: ToolCall<'_>) -> Value {
    let mut params = call.params.clone();
    let Some(object) = params.as_object_mut() else {
        return params;
    };

    let node_type = match super::canonical(call.method) {
        "create_query_node" => NodeType::Query,
        "create_vars_node" => NodeType::Variable,
        "create_text_node" => NodeType::Text,
        "create_page" => {
            object.entry("order").or_insert(Value::from(0));
            return params;
        }
        _ => return params,
    };

    let size = node_type.default_size();
    if object.get("position").is_none_or(Value::is_null) {
        let origin = beside(document, agent, node_type);
        object.insert(
            "position".to_string(),
            Value::from(vec![origin.x, origin.y]),
        );
    }
    if node_type == NodeType::Text {
        object.entry("height").or_insert(Value::from(size.height));
    } else if object.get("size").is_none_or(Value::is_null) {
        object.insert(
            "size".to_string(),
            Value::from(vec![size.width, size.height]),
        );
    }
    params
}

/// A free slot to the agent's right. The row is the agent's outgoing-edge count, so successive
/// query nodes stack downwards instead of landing on top of each other.
fn beside(document: &Document, agent: &NodeId, node_type: NodeType) -> Point {
    let Some(node) = document.node(agent) else {
        return Point::new(0.0, 0.0);
    };
    // Saturating rather than casting: a page with more than 2^32 edges is not a real page, and
    // the conversion has to be exact for the row pitch to be.
    let row = u32::try_from(
        document
            .edges()
            .iter()
            .filter(|edge| &edge.source == agent)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let height = node_type.default_size().height;
    Point::new(
        node.position.x + node.size().width + GUTTER,
        node.position.y + f64::from(row) * (height + ROW_GAP),
    )
}
