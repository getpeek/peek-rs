//! `pageDelta.ts`: the id-keyed diff a delta entry stores, and its replay.

use std::collections::HashSet;
use std::hash::Hash;

use super::entry::{ChangeSummary, PageDelta, PageSnapshot};

struct Diff<T, K> {
    puts: Vec<T>,
    deleted: Vec<K>,
    added: u32,
    edited: u32,
}

impl<T, K> Diff<T, K> {
    fn deleted_count(&self) -> u32 {
        u32::try_from(self.deleted.len()).unwrap_or(u32::MAX)
    }
}

fn diff_by_id<T: Clone + PartialEq, K: Clone + Eq + Hash>(
    previous: &[T],
    next: &[T],
    id: impl Fn(&T) -> &K,
) -> Diff<T, K> {
    let next_ids: HashSet<&K> = next.iter().map(&id).collect();
    let mut diff = Diff {
        puts: Vec::new(),
        deleted: Vec::new(),
        added: 0,
        edited: 0,
    };
    for item in next {
        match previous.iter().find(|old| id(old) == id(item)) {
            None => diff.added += 1,
            Some(old) if old != item => diff.edited += 1,
            Some(_) => continue,
        }
        diff.puts.push(item.clone());
    }
    diff.deleted = previous
        .iter()
        .map(&id)
        .filter(|old| !next_ids.contains(old))
        .cloned()
        .collect();
    diff
}

/// What changed from `previous` to `next`, as a delta and the counts the timeline shows.
#[must_use]
pub fn diff(previous: &PageSnapshot, next: &PageSnapshot) -> (PageDelta, ChangeSummary) {
    let nodes = diff_by_id(&previous.nodes, &next.nodes, |node| &node.id);
    let edges = diff_by_id(&previous.edges, &next.edges, |edge| &edge.id);
    let regions = diff_by_id(&previous.regions, &next.regions, |region| &region.id);
    let renamed = previous.name != next.name;
    let summary = ChangeSummary {
        added_nodes: nodes.added,
        edited_nodes: nodes.edited,
        removed_nodes: nodes.deleted_count(),
        added_edges: edges.added,
        edited_edges: edges.edited,
        removed_edges: edges.deleted_count(),
        changed_regions: regions.added + regions.edited + regions.deleted_count(),
        renamed,
    };
    let delta = PageDelta {
        put_nodes: nodes.puts,
        del_node_ids: nodes.deleted,
        put_edges: edges.puts,
        del_edge_ids: edges.deleted,
        put_regions: regions.puts,
        del_region_ids: regions.deleted,
        name: renamed.then(|| next.name.clone()),
    };
    (delta, summary)
}

/// Deletes first, then upserts — an upsert keeps the item's place, as a JS `Map.set` does.
fn apply_by_id<T: Clone, K: Eq>(
    items: &mut Vec<T>,
    puts: &[T],
    deleted: &[K],
    id: impl Fn(&T) -> &K,
) {
    items.retain(|item| !deleted.contains(id(item)));
    for put in puts {
        match items.iter_mut().find(|item| id(item) == id(put)) {
            Some(existing) => *existing = put.clone(),
            None => items.push(put.clone()),
        }
    }
}

#[must_use]
pub fn apply(mut snapshot: PageSnapshot, delta: &PageDelta) -> PageSnapshot {
    apply_by_id(
        &mut snapshot.nodes,
        &delta.put_nodes,
        &delta.del_node_ids,
        |node| &node.id,
    );
    apply_by_id(
        &mut snapshot.edges,
        &delta.put_edges,
        &delta.del_edge_ids,
        |edge| &edge.id,
    );
    apply_by_id(
        &mut snapshot.regions,
        &delta.put_regions,
        &delta.del_region_ids,
        |region| &region.id,
    );
    if let Some(name) = &delta.name {
        snapshot.name.clone_from(name);
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::Edge;
    use crate::geometry::Point;
    use crate::ids::NodeId;
    use crate::node::{Node, NodeKind, TextData};

    fn text(id: &str, body: &str) -> Node {
        Node {
            id: NodeId::from(id),
            position: Point::new(0.0, 0.0),
            width: None,
            height: None,
            measured: None,
            selected: false,
            kind: NodeKind::Text(TextData {
                text: body.to_string(),
            }),
        }
    }

    fn page(name: &str, nodes: Vec<Node>) -> PageSnapshot {
        PageSnapshot {
            nodes,
            ..PageSnapshot::empty(name)
        }
    }

    #[test]
    fn counts_added_edited_and_removed_items() {
        let before = page("A", vec![text("a", "1"), text("b", "2"), text("c", "3")]);
        let mut after = page(
            "B",
            vec![text("a", "1"), text("b", "changed"), text("d", "4")],
        );
        after
            .edges
            .push(Edge::between(NodeId::from("a"), NodeId::from("b")));

        let (delta, summary) = diff(&before, &after);

        assert_eq!(summary.added_nodes, 1);
        assert_eq!(summary.edited_nodes, 1);
        assert_eq!(summary.removed_nodes, 1);
        assert_eq!(summary.added_edges, 1);
        assert!(summary.renamed);
        assert_eq!(delta.del_node_ids, vec![NodeId::from("c")]);
        assert_eq!(delta.name.as_deref(), Some("B"));
        assert_eq!(apply(before, &delta), after);
    }

    #[test]
    fn an_unchanged_page_diffs_to_nothing() {
        let snapshot = page("A", vec![text("a", "1")]);
        let (delta, summary) = diff(&snapshot, &snapshot);
        assert_eq!(delta, PageDelta::default());
        assert_eq!(summary.change_count(), 0);
    }

    #[test]
    fn an_upsert_keeps_the_item_in_place() {
        let before = page("A", vec![text("a", "1"), text("b", "2")]);
        let after = page("A", vec![text("a", "changed"), text("b", "2")]);
        let (delta, _) = diff(&before, &after);
        let replayed = apply(before, &delta);
        assert_eq!(replayed.nodes[0].id, NodeId::from("a"));
        assert_eq!(replayed, after);
    }
}
