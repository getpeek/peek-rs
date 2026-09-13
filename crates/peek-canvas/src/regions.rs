//! Regions: named, coloured sets of nodes used for wayfinding when zoomed out.
//!
//! Membership is exclusive, which is the whole of the complexity here. Grouping a node claims
//! it from whatever region held it before, and a region left empty by that claim is deleted
//! rather than lingering as a label over nothing — `~/labs/peek/src/mcp/regionTools.ts`.
//!
//! `history::Snapshot` already captures `page.regions`, so undo works without a history change.

use peek_document::{NodeId, REGION_COLOR_COUNT, Region, RegionId, RegionStatus};

use crate::history::EditKind;
use crate::model::Document;

/// What a caller chooses when grouping; the id and the colour are the document's to assign.
#[derive(Debug, Clone)]
pub struct NewRegion {
    pub name: String,
    pub desc: String,
    pub status: RegionStatus,
}

impl Document {
    /// Groups `members` into a new region on the active page, claiming them from any region
    /// that already holds them.
    ///
    /// The colour is the surviving region count modulo the palette, so a page's regions cycle
    /// through it in creation order.
    pub fn group_nodes(&mut self, members: Vec<NodeId>, region: NewRegion) -> RegionId {
        let id = RegionId::generate();
        self.begin(EditKind::Structure);
        let regions = &mut self.active_page_mut().regions;
        claim(regions, &members, None);
        let color_index = u8::try_from(regions.len() % REGION_COLOR_COUNT).unwrap_or(0);
        regions.push(Region {
            id: id.clone(),
            name: region.name,
            desc: region.desc,
            color_index,
            status: region.status,
            member_ids: members,
        });
        self.touch();
        id
    }

    /// Adds `members` to an existing region on the active page, claiming them the same way.
    /// The region's name, description and status are left alone. `false` when it is not here.
    pub fn add_to_region(&mut self, region: &RegionId, members: Vec<NodeId>) -> bool {
        if !self.has_region(region) {
            return false;
        }
        self.begin(EditKind::Structure);
        let regions = &mut self.active_page_mut().regions;
        claim(regions, &members, Some(region));
        if let Some(target) = regions.iter_mut().find(|candidate| &candidate.id == region) {
            for member in members {
                if !target.member_ids.contains(&member) {
                    target.member_ids.push(member);
                }
            }
        }
        self.touch();
        true
    }

    /// Deletes a region on the active page. Its members are untouched — they become ungrouped.
    pub fn remove_region(&mut self, region: &RegionId) -> bool {
        if !self.has_region(region) {
            return false;
        }
        self.begin(EditKind::Structure);
        self.active_page_mut()
            .regions
            .retain(|candidate| &candidate.id != region);
        self.touch();
        true
    }

    /// The active page's regions, in creation order.
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.active_page().regions
    }

    #[must_use]
    fn has_region(&self, region: &RegionId) -> bool {
        self.regions()
            .iter()
            .any(|candidate| &candidate.id == region)
    }
}

/// Strips `members` from every region but `keep`, then drops any region the strip emptied.
///
/// Kept separate from the two callers because the emptied-region sweep has to run after the
/// whole strip, not per region: a region can lose its last member to the second claim.
fn claim(regions: &mut Vec<Region>, members: &[NodeId], keep: Option<&RegionId>) {
    for region in regions.iter_mut() {
        if keep.is_some_and(|keep| &region.id == keep) {
            continue;
        }
        region.member_ids.retain(|member| !members.contains(member));
    }
    regions.retain(|region| {
        keep.is_some_and(|keep| &region.id == keep) || !region.member_ids.is_empty()
    });
}

#[cfg(test)]
mod tests {
    use super::NewRegion;
    use crate::Document;
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{CanvasDocument, Node, NodeId, NodeKind, QueryData, RegionStatus};

    fn document() -> Document {
        let mut persisted = CanvasDocument::empty();
        let page = persisted
            .pages
            .get_mut(&persisted.active_page_id)
            .expect("the empty document has one page");
        for name in ["a", "b", "c", "d", "e", "f", "g"] {
            let mut node = Node::new(
                peek_document::NodeType::Query,
                Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 50.0)),
            );
            node.id = NodeId::from(name);
            node.kind = NodeKind::Query(QueryData::default());
            page.nodes.push(node);
        }
        Document::load(persisted)
    }

    fn suggested(name: &str) -> NewRegion {
        NewRegion {
            name: name.to_string(),
            desc: String::new(),
            status: RegionStatus::Suggested,
        }
    }

    fn ids(names: &[&str]) -> Vec<NodeId> {
        names.iter().map(|name| NodeId::from(*name)).collect()
    }

    #[test]
    fn grouping_claims_members_from_the_region_that_held_them() {
        let mut document = document();
        let first = document.group_nodes(ids(&["a", "b"]), suggested("first"));
        document.group_nodes(ids(&["b", "c"]), suggested("second"));

        let kept = document
            .regions()
            .iter()
            .find(|region| region.id == first)
            .expect("the first region still has a member");
        assert_eq!(kept.member_ids, ids(&["a"]));
    }

    /// A label over nothing is worse than no label, so the claim that empties a region takes it.
    #[test]
    fn a_region_emptied_by_a_claim_is_deleted() {
        let mut document = document();
        document.group_nodes(ids(&["a", "b"]), suggested("first"));
        document.group_nodes(ids(&["a", "b"]), suggested("second"));

        assert_eq!(document.regions().len(), 1);
        assert_eq!(document.regions()[0].name, "second");
    }

    #[test]
    fn colours_cycle_through_the_palette() {
        let mut document = document();
        for name in ["a", "b", "c", "d", "e", "f", "g"] {
            document.group_nodes(ids(&[name]), suggested(name));
        }
        let colors: Vec<u8> = document
            .regions()
            .iter()
            .map(|region| region.color_index)
            .collect();
        assert_eq!(colors, vec![0, 1, 2, 3, 4, 0, 1]);
    }

    #[test]
    fn adding_to_a_region_keeps_its_name_and_claims_the_nodes() {
        let mut document = document();
        let first = document.group_nodes(ids(&["a"]), suggested("first"));
        let second = document.group_nodes(ids(&["b"]), suggested("second"));

        assert!(document.add_to_region(&first, ids(&["b", "c"])));

        let first = document
            .regions()
            .iter()
            .find(|region| region.id == first)
            .expect("the target survives");
        assert_eq!(first.name, "first");
        assert_eq!(first.member_ids, ids(&["a", "b", "c"]));
        assert!(
            !document.regions().iter().any(|region| region.id == second),
            "the region the claim emptied is gone"
        );
    }

    #[test]
    fn adding_the_same_node_twice_does_not_duplicate_it() {
        let mut document = document();
        let region = document.group_nodes(ids(&["a"]), suggested("first"));
        assert!(document.add_to_region(&region, ids(&["a", "b"])));

        assert_eq!(document.regions()[0].member_ids, ids(&["a", "b"]));
    }

    #[test]
    fn removing_a_region_keeps_its_members() {
        let mut document = document();
        let region = document.group_nodes(ids(&["a", "b"]), suggested("first"));

        assert!(document.remove_region(&region));
        assert!(document.regions().is_empty());
        assert!(document.node(&NodeId::from("a")).is_some());
    }

    #[test]
    fn an_unknown_region_is_refused_rather_than_ignored() {
        let mut document = document();
        let absent = peek_document::RegionId::from("region_nope");

        assert!(!document.add_to_region(&absent, ids(&["a"])));
        assert!(!document.remove_region(&absent));
    }

    #[test]
    fn grouping_is_one_undo_step() {
        let mut document = document();
        document.group_nodes(ids(&["a", "b"]), suggested("first"));
        document.checkpoint();

        assert!(document.undo());
        assert!(document.regions().is_empty());
    }
}
