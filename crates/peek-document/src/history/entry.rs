use serde::{Deserialize, Serialize};

use crate::document::Page;
use crate::edge::Edge;
use crate::ids::{CheckpointId, EdgeId, NodeId, PageId, RegionId};
use crate::node::Node;
use crate::region::Region;

/// One page's content at a checkpoint. The viewport is deliberately absent: scrubbing frames
/// the version rather than restoring where the camera was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageSnapshot {
    pub name: String,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Optional on read because entries written before regions existed carry none.
    #[serde(default)]
    pub regions: Vec<Region>,
}

impl PageSnapshot {
    /// The page as the log would give it back: the fields a write drops (`selected`,
    /// `measured`) are cleared, so a snapshot of an unchanged page equals the tail parsed from
    /// disk and records nothing.
    #[must_use]
    pub fn of(page: &Page) -> Self {
        let nodes = page
            .nodes
            .iter()
            .map(|node| Node {
                selected: false,
                measured: None,
                ..node.clone()
            })
            .collect();
        let edges = page
            .edges
            .iter()
            .map(|edge| Edge {
                selected: false,
                ..edge.clone()
            })
            .collect();
        Self {
            name: page.name.clone(),
            nodes,
            edges,
            regions: page.regions.clone(),
        }
    }

    #[must_use]
    pub fn empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            regions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageDelta {
    #[serde(default)]
    pub put_nodes: Vec<Node>,
    #[serde(default)]
    pub del_node_ids: Vec<NodeId>,
    #[serde(default)]
    pub put_edges: Vec<Edge>,
    #[serde(default)]
    pub del_edge_ids: Vec<EdgeId>,
    #[serde(default)]
    pub put_regions: Vec<Region>,
    #[serde(default)]
    pub del_region_ids: Vec<RegionId>,
    /// Present only when the page was renamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChangeSummary {
    pub added_nodes: u32,
    pub edited_nodes: u32,
    pub removed_nodes: u32,
    pub added_edges: u32,
    pub edited_edges: u32,
    pub removed_edges: u32,
    pub changed_regions: u32,
    pub renamed: bool,
}

impl ChangeSummary {
    /// `changeCount` in `format.ts`: what the timeline sizes a dot by.
    #[must_use]
    pub fn change_count(&self) -> u32 {
        self.added_nodes
            + self.edited_nodes
            + self.removed_nodes
            + self.added_edges
            + self.edited_edges
            + self.removed_edges
            + self.changed_regions
            + u32::from(self.renamed)
    }

    /// `describeSummary` in `format.ts`.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        for (count, verb) in [
            (self.added_nodes, "added"),
            (self.edited_nodes, "edited"),
            (self.removed_nodes, "removed"),
        ] {
            if count > 0 {
                parts.push(format!("{count} {} {verb}", plural(count, "node")));
            }
        }
        let edges = self.added_edges + self.edited_edges + self.removed_edges;
        if edges > 0 {
            parts.push(format!("{edges} {}", plural(edges, "connection")));
        }
        if self.changed_regions > 0 {
            let regions = self.changed_regions;
            parts.push(format!("{regions} {}", plural(regions, "region")));
        }
        if self.renamed {
            parts.push("renamed".to_string());
        }
        if parts.is_empty() {
            return "No structural changes".to_string();
        }
        parts.join(" · ")
    }
}

fn plural(count: u32, noun: &str) -> String {
    if count == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EntryBody {
    Full { snapshot: PageSnapshot },
    Delta { delta: PageDelta },
}

/// One line of `<connection>.history.jsonl`, as `src/canvas/history/types.ts` defines it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawEntry")]
pub struct HistoryEntry {
    pub id: CheckpointId,
    /// The previous entry for the same page; `None` starts a chain. Replay trusts a delta only
    /// when this links to the entry before it, the way git trusts a parent commit.
    pub parent_id: Option<CheckpointId>,
    pub page_id: PageId,
    /// 1-based and persisted, so "Version N" stays stable when compaction drops old entries.
    pub seq: u32,
    /// Milliseconds since the epoch, as `Date.now()` records it.
    pub taken_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub summary: ChangeSummary,
    #[serde(flatten)]
    pub body: EntryBody,
}

impl HistoryEntry {
    #[must_use]
    pub fn is_full(&self) -> bool {
        matches!(self.body, EntryBody::Full { .. })
    }
}

/// The wire shape, deserialized without `flatten`: a flattened, internally tagged body makes
/// serde buffer every line into an untyped tree first, and the logs run to megabytes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEntry {
    id: CheckpointId,
    parent_id: Option<CheckpointId>,
    page_id: PageId,
    seq: u32,
    taken_at: i64,
    #[serde(default)]
    label: Option<String>,
    summary: ChangeSummary,
    kind: RawKind,
    #[serde(default)]
    snapshot: Option<PageSnapshot>,
    #[serde(default)]
    delta: Option<PageDelta>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawKind {
    Full,
    Delta,
}

impl TryFrom<RawEntry> for HistoryEntry {
    type Error = &'static str;

    fn try_from(raw: RawEntry) -> Result<Self, Self::Error> {
        let body = match raw.kind {
            RawKind::Full => EntryBody::Full {
                snapshot: raw.snapshot.ok_or("a full entry without a snapshot")?,
            },
            RawKind::Delta => EntryBody::Delta {
                delta: raw.delta.ok_or("a delta entry without a delta")?,
            },
        };
        Ok(Self {
            id: raw.id,
            parent_id: raw.parent_id,
            page_id: raw.page_id,
            seq: raw.seq,
            taken_at: raw.taken_at,
            label: raw.label,
            summary: raw.summary,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like the first line of a real log the TypeScript app wrote.
    const FULL_LINE: &str = r#"{"id":"chk_ab12cd34","parentId":null,"pageId":"page_1","seq":1,"takenAt":1727000000000,"summary":{"addedNodes":1,"editedNodes":0,"removedNodes":0,"addedEdges":0,"editedEdges":0,"removedEdges":0,"changedRegions":0,"renamed":false},"kind":"full","snapshot":{"name":"Page 1","nodes":[{"id":"text_1","type":"text","position":{"x":10,"y":20},"width":200,"height":80,"data":{"text":"hello"},"selected":true,"measured":{"width":200,"height":80}}],"edges":[],"regions":[]}}"#;
    const DELTA_LINE: &str = r#"{"id":"chk_ef56gh78","parentId":"chk_ab12cd34","pageId":"page_1","seq":2,"takenAt":1727000030000,"label":"Restored Version 1","summary":{"addedNodes":0,"editedNodes":1,"removedNodes":0,"addedEdges":0,"editedEdges":0,"removedEdges":0,"renamed":false},"kind":"delta","delta":{"putNodes":[],"delNodeIds":["text_1"],"putEdges":[],"delEdgeIds":[]}}"#;

    #[test]
    fn reads_both_kinds_of_line() {
        let full: HistoryEntry = serde_json::from_str(FULL_LINE).unwrap();
        assert!(full.is_full());
        assert_eq!(full.parent_id, None);
        let delta: HistoryEntry = serde_json::from_str(DELTA_LINE).unwrap();
        assert_eq!(delta.label.as_deref(), Some("Restored Version 1"));
        let EntryBody::Delta { delta } = delta.body else {
            panic!("expected a delta");
        };
        assert_eq!(delta.del_node_ids, vec![NodeId::from("text_1")]);
        assert!(delta.put_regions.is_empty());
    }

    #[test]
    fn writes_the_typescript_key_order_and_drops_ephemeral_fields() {
        let full: HistoryEntry = serde_json::from_str(FULL_LINE).unwrap();
        let line = serde_json::to_string(&full).unwrap();
        assert!(line.starts_with(r#"{"id":"chk_ab12cd34","parentId":null,"pageId":"page_1","seq":1,"takenAt":1727000000000,"summary":"#));
        assert!(line.contains(r#""kind":"full","snapshot":{"name":"Page 1""#));
        assert!(!line.contains("selected"));
        assert!(!line.contains("measured"));
        assert!(!line.contains("label"));
        let back: HistoryEntry = serde_json::from_str(&line).unwrap();
        assert_eq!(serde_json::to_string(&back).unwrap(), line);
    }

    #[test]
    fn a_full_entry_without_a_snapshot_is_rejected() {
        let line = r#"{"id":"chk_1","parentId":null,"pageId":"p","seq":1,"takenAt":1,"summary":{},"kind":"full"}"#;
        assert!(serde_json::from_str::<HistoryEntry>(line).is_err());
    }

    #[test]
    fn describes_a_summary_as_format_ts_does() {
        let summary = ChangeSummary {
            added_nodes: 2,
            removed_nodes: 1,
            added_edges: 1,
            changed_regions: 1,
            renamed: true,
            ..ChangeSummary::default()
        };
        assert_eq!(
            summary.describe(),
            "2 nodes added · 1 node removed · 1 connection · 1 region · renamed"
        );
        assert_eq!(summary.change_count(), 6);
        assert_eq!(ChangeSummary::default().describe(), "No structural changes");
    }
}
