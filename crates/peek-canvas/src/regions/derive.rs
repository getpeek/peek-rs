//! Resolving a region against the live nodes — the port of `regionGeometry.ts:deriveRegions`.
//!
//! A region carries no geometry of its own, so its box is recomputed every frame from the
//! members that still exist. That is what lets deletion and undo stay simple: `member_ids` is
//! allowed to name nodes that are gone, and nothing has to walk the regions to fix them up.

use peek_document::geometry::Rect;
use peek_document::{Node, NodeId, Region, RegionId, RegionStatus};

/// How far a region's box stands off its members, on every side.
pub const REGION_PADDING: f64 = 56.0;

/// A region with its live members and the box they span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub id: RegionId,
    pub name: String,
    pub desc: String,
    pub color_index: u8,
    pub status: RegionStatus,
    pub member_ids: Vec<NodeId>,
}

impl Derived {
    /// Recomputed rather than stored: `bbox` would go stale the moment a member moved.
    #[must_use]
    pub fn bbox(&self, nodes: &[Node]) -> Rect {
        bounds(nodes, &self.member_ids).unwrap_or_default()
    }
}

/// One entry per region that still has a member on the page, in document order.
///
/// A region whose every member is gone is skipped rather than returned empty: it has no box,
/// so it has nowhere on the canvas to be.
#[must_use]
pub fn derive(nodes: &[Node], regions: &[Region]) -> Vec<Derived> {
    regions
        .iter()
        .filter_map(|region| {
            let member_ids: Vec<NodeId> = region
                .member_ids
                .iter()
                .filter(|id| nodes.iter().any(|node| &&node.id == id))
                .cloned()
                .collect();
            (!member_ids.is_empty()).then(|| Derived {
                id: region.id.clone(),
                name: region.name.clone(),
                desc: region.desc.clone(),
                color_index: region.color_index,
                status: region.status,
                member_ids,
            })
        })
        .collect()
}

/// The padded union of the named nodes' boxes, or `None` when none of them are here.
#[must_use]
pub fn bounds(nodes: &[Node], members: &[NodeId]) -> Option<Rect> {
    nodes
        .iter()
        .filter(|node| members.contains(&node.id))
        .map(Node::bounds)
        .reduce(Rect::union)
        .map(|box_| box_.dilated(REGION_PADDING))
}

#[cfg(test)]
mod tests {
    use super::{REGION_PADDING, bounds, derive};
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{Node, NodeId, NodeType, Region, RegionId, RegionStatus};

    /// A `Text` node, because `Node::new` clamps to the kind's minimum and Text's is the
    /// smallest — the numbers below are then the ones written here.
    fn node(id: &str, x: f64, y: f64) -> Node {
        let mut node = Node::new(
            NodeType::Text,
            Rect::new(Point::new(x, y), Size::new(100.0, 50.0)),
        );
        node.id = NodeId::from(id);
        node
    }

    fn region(id: &str, members: &[&str]) -> Region {
        Region {
            id: RegionId::from(id),
            name: id.to_string(),
            desc: String::new(),
            color_index: 0,
            status: RegionStatus::Confirmed,
            member_ids: members.iter().map(|id| NodeId::from(*id)).collect(),
        }
    }

    #[test]
    fn the_box_is_the_members_union_padded_on_every_side() {
        let nodes = vec![node("a", 0.0, 0.0), node("b", 400.0, 200.0)];
        let derived = derive(&nodes, &[region("r", &["a", "b"])]);

        let box_ = derived[0].bbox(&nodes);
        assert_eq!(box_.min(), Point::new(-REGION_PADDING, -REGION_PADDING));
        assert_eq!(
            box_.max(),
            Point::new(500.0 + REGION_PADDING, 250.0 + REGION_PADDING)
        );
    }

    /// `member_ids` is allowed to name deleted nodes, so the filter is the only thing keeping
    /// a stale id from dragging the box back to wherever that node used to be.
    #[test]
    fn a_member_that_no_longer_exists_is_dropped() {
        let nodes = vec![node("a", 0.0, 0.0)];
        let derived = derive(&nodes, &[region("r", &["a", "ghost"])]);

        assert_eq!(derived[0].member_ids, vec![NodeId::from("a")]);
    }

    #[test]
    fn a_region_with_no_live_members_is_skipped() {
        let nodes = vec![node("a", 0.0, 0.0)];

        assert!(derive(&nodes, &[region("r", &["ghost"])]).is_empty());
    }

    #[test]
    fn bounds_of_nothing_is_none() {
        assert!(bounds(&[node("a", 0.0, 0.0)], &[NodeId::from("ghost")]).is_none());
    }

    #[test]
    fn regions_keep_document_order() {
        let nodes = vec![node("a", 0.0, 0.0), node("b", 10.0, 0.0)];
        let derived = derive(&nodes, &[region("second", &["b"]), region("first", &["a"])]);

        let names: Vec<&str> = derived.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["second", "first"]);
    }
}
