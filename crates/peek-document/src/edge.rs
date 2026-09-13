use serde::{Deserialize, Serialize};

use crate::ids::{EdgeId, NodeId};

/// Peek authors only `{ id, source, target }`; the `floating` edge type is applied at render
/// time, so `type` is normally absent on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub edge_type: Option<String>,
    #[serde(default, skip_serializing)]
    pub selected: bool,
}

impl Edge {
    #[must_use]
    pub fn between(source: NodeId, target: NodeId) -> Self {
        Self {
            id: EdgeId::between(&source, &target),
            source,
            target,
            edge_type: None,
            selected: false,
        }
    }
}
