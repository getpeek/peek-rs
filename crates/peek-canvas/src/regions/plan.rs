//! What ⌘G and ⌘⇧G would do to the current selection — the port of `useGroupSelection.ts` and
//! `useUngroupSelection.ts`.
//!
//! Kept pure and away from the mutators so the command registry can ask "is this available?"
//! per frame without touching the document, and so the folding rule is testable without a
//! window.

use peek_document::{NodeId, RegionId};

use crate::model::Document;

/// What grouping the selection would mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupPlan {
    /// Nothing to do: fewer than two nodes, or the selection is already exactly one region.
    Unavailable,
    /// The selection touches no region, or several — mint a new one.
    Create,
    /// The selection touches exactly one region, so it grows rather than splitting in two.
    /// That is how a region is extended by hand, and it keeps the region's name.
    FoldInto(RegionId),
}

impl Document {
    /// What `Region::GroupSelection` would do right now.
    #[must_use]
    pub fn group_plan(&self) -> GroupPlan {
        if self.selected().len() < 2 {
            return GroupPlan::Unavailable;
        }
        let selected: Vec<&NodeId> = self.selected().iter().collect();
        let mut touched = self
            .regions()
            .iter()
            .filter(|region| region.member_ids.iter().any(|id| selected.contains(&id)));

        let Some(only) = touched.next() else {
            return GroupPlan::Create;
        };
        if touched.next().is_some() {
            return GroupPlan::Create;
        }
        // Everything selected already sits in that one region — there is nothing to fold in.
        if selected.iter().all(|id| only.member_ids.contains(id)) {
            return GroupPlan::Unavailable;
        }
        GroupPlan::FoldInto(only.id.clone())
    }

    /// The selected nodes that some region holds — what `Region::UngroupSelection` would pull
    /// out. Empty means the command has nothing to do.
    #[must_use]
    pub fn grouped_selection(&self) -> Vec<NodeId> {
        self.selected()
            .iter()
            .filter(|id| {
                self.regions()
                    .iter()
                    .any(|region| region.member_ids.contains(id))
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::GroupPlan;
    use crate::regions::tests::{document, ids, suggested};
    use peek_document::NodeId;

    fn select(document: &mut crate::Document, names: &[&str]) {
        document.select_only(names.iter().map(|name| NodeId::from(*name)));
    }

    #[test]
    fn one_node_is_not_a_group() {
        let mut document = document();
        select(&mut document, &["a"]);

        assert_eq!(document.group_plan(), GroupPlan::Unavailable);
    }

    #[test]
    fn an_ungrouped_selection_creates_a_region() {
        let mut document = document();
        select(&mut document, &["a", "b"]);

        assert_eq!(document.group_plan(), GroupPlan::Create);
    }

    #[test]
    fn a_selection_touching_one_region_folds_into_it() {
        let mut document = document();
        let first = document.group_nodes(ids(&["a", "b"]), suggested("first"));
        select(&mut document, &["b", "c"]);

        assert_eq!(document.group_plan(), GroupPlan::FoldInto(first));
    }

    /// Folding two regions into each other would have to pick a name to keep, so the reference
    /// mints a third instead and hands naming to the user.
    #[test]
    fn a_selection_touching_two_regions_creates_a_third() {
        let mut document = document();
        document.group_nodes(ids(&["a"]), suggested("first"));
        document.group_nodes(ids(&["b"]), suggested("second"));
        select(&mut document, &["a", "b"]);

        assert_eq!(document.group_plan(), GroupPlan::Create);
    }

    #[test]
    fn re_grouping_a_regions_own_members_does_nothing() {
        let mut document = document();
        document.group_nodes(ids(&["a", "b"]), suggested("first"));
        select(&mut document, &["a", "b"]);

        assert_eq!(document.group_plan(), GroupPlan::Unavailable);
    }

    #[test]
    fn ungrouping_only_names_the_selected_nodes_a_region_holds() {
        let mut document = document();
        document.group_nodes(ids(&["a", "b"]), suggested("first"));
        select(&mut document, &["b", "c"]);

        assert_eq!(document.grouped_selection(), ids(&["b"]));
    }

    #[test]
    fn an_ungrouped_selection_has_nothing_to_ungroup() {
        let mut document = document();
        select(&mut document, &["a", "b"]);

        assert!(document.grouped_selection().is_empty());
    }
}
