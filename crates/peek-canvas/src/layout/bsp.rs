//! `Zoom::FitSelection`: tile the selected nodes across the viewport at 100% zoom.
//!
//! A port of `~/labs/peek/src/canvas/hooks/bspTiles.ts`, driven by
//! `CanvasApi.fitSelectedToViewport` in `useCanvas.ts`. This is a *layout*, not a camera move:
//! the reference writes each selected node's position and size, and only then snaps the viewport
//! to zoom 1. The camera half stays in the view, which is the only place that knows the pane.
//!
//! The constants and the split rule are the reference's. A tiling that divides differently is a
//! different tiling, and the two apps open the same documents.

use peek_document::NodeId;
use peek_document::geometry::{Point, Rect, Size};

use crate::history::EditKind;
use crate::model::Document;

/// Breathing room around the whole layout, in world units — screen pixels at zoom 1.
pub const FIT_PADDING: f64 = 24.0;
/// And between two tiles across a split.
pub const FIT_GAP: f64 = 16.0;

/// Partitions `region` into `count` roughly-equal-area tiles.
///
/// Each split runs along the region's longer axis and divides it in proportion to how many tiles
/// fall on each side, which is what keeps tiles close to square without ever measuring one: 2
/// halve the region, 3 take two thirds and then subdivide, 4 are quarters.
///
/// Depth-first, first half emitted in full before the second, so a tile's index in `out` is the
/// index of the node it belongs to. `out` rather than a returned `Vec` for exactly that reason —
/// the emit order *is* the contract.
fn tiles(region: Rect, count: u32, gap: f64, out: &mut Vec<Rect>) {
    if count <= 1 {
        out.push(region);
        return;
    }

    let first = count.div_ceil(2);
    let second = count - first;
    let share = f64::from(first) / f64::from(count);

    // The gap comes out of the axis once, before the proportional divide — so a split always
    // costs one gap however lopsided it is.
    if region.size.width >= region.size.height {
        let available = region.size.width - gap;
        let first_width = available * share;
        let origin = Point::new(region.origin.x + first_width + gap, region.origin.y);
        tiles(
            Rect::new(region.origin, Size::new(first_width, region.size.height)),
            first,
            gap,
            out,
        );
        tiles(
            Rect::new(
                origin,
                Size::new(available - first_width, region.size.height),
            ),
            second,
            gap,
            out,
        );
        return;
    }

    let available = region.size.height - gap;
    let first_height = available * share;
    let origin = Point::new(region.origin.x, region.origin.y + first_height + gap);
    tiles(
        Rect::new(region.origin, Size::new(region.size.width, first_height)),
        first,
        gap,
        out,
    );
    tiles(
        Rect::new(
            origin,
            Size::new(region.size.width, available - first_height),
        ),
        second,
        gap,
        out,
    );
}

/// Lays the selection out to fill the pane, as one undo step. Returns whether anything moved.
///
/// `pane` is the usable pane — the window minus the chrome the title bar overlays — and `center`
/// the world point the camera is looking at now. At zoom 1 one world unit is one pixel, so the
/// pane maps to a `pane`-sized world rectangle centred there, and the caller's job afterwards is
/// simply to put the camera at zoom 1 over that same point.
pub fn fit_selection(document: &mut Document, pane: Size, center: Point) -> bool {
    // Document order, not selection order: `Document::selected` is a `BTreeSet` sorted by id
    // string, and the reference assigns tiles in `rf.getNodes()` order, which is the page's node
    // array. Same nodes, different tiles.
    let ids: Vec<NodeId> = document
        .nodes()
        .iter()
        .filter(|node| document.is_selected(&node.id))
        .map(|node| node.id.clone())
        .collect();
    let Ok(count) = u32::try_from(ids.len()) else {
        return false;
    };
    if count == 0 {
        return false;
    }

    let origin = Point::new(center.x - pane.width / 2.0, center.y - pane.height / 2.0);
    let region = Rect::new(origin, pane).dilated(-FIT_PADDING);

    let mut placements = Vec::with_capacity(ids.len());
    tiles(region, count, FIT_GAP, &mut placements);

    // `EditKind::Structure` never coalesces, so the fit is its own press to undo rather than the
    // tail of whatever drag or resize preceded it.
    document.transaction_of(EditKind::Structure, |document| {
        for (id, tile) in ids.iter().zip(placements) {
            document.set_bounds(id, tile);
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::{FIT_GAP, FIT_PADDING, fit_selection, tiles};
    use crate::model::Document;
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{CanvasDocument, NodeId, NodeType};

    /// A 1200×800 window with nothing to scroll past, so the padded region is 1152×752 at the
    /// origin — the numbers every case below is written against.
    const PANE: Size = Size::new(1200.0, 800.0);

    fn region() -> Rect {
        Rect::new(Point::new(-600.0, -400.0), PANE).dilated(-FIT_PADDING)
    }

    fn tiled(count: u32) -> Vec<Rect> {
        let mut out = Vec::new();
        tiles(region(), count, FIT_GAP, &mut out);
        out
    }

    fn assert_close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() < 0.001,
            "{what}: {actual} is not {expected}"
        );
    }

    #[test]
    fn one_tile_is_the_whole_region() {
        assert_eq!(tiled(1), vec![region()]);
    }

    /// A wide region splits side by side, a tall one top to bottom, and either way the gap comes
    /// out of the axis exactly once — the halves are equal, not each short by a full gap.
    #[test]
    fn two_tiles_split_the_longer_axis() {
        let wide = tiled(2);
        assert_close(wide[0].size.width, 568.0, "the left half");
        assert_close(wide[1].size.width, 568.0, "the right half");
        assert_close(wide[0].size.height, 752.0, "full height");
        assert_close(wide[1].origin.x - wide[0].max().x, FIT_GAP, "one gap");

        let mut tall = Vec::new();
        tiles(
            Rect::new(Point::new(0.0, 0.0), Size::new(400.0, 1000.0)),
            2,
            FIT_GAP,
            &mut tall,
        );
        assert_close(tall[0].size.height, 492.0, "the top half");
        assert_close(tall[0].size.width, 400.0, "full width");
        assert_close(tall[1].origin.y - tall[0].max().y, FIT_GAP, "one gap");
    }

    /// `first = ceil(count / 2)`, so an odd count gives the *first* subtree the larger share:
    /// three tiles are two thirds split again, then one third whole.
    #[test]
    fn three_tiles_are_two_thirds_then_subdivided() {
        let tiles = tiled(3);
        assert_eq!(tiles.len(), 3);
        assert_close(
            tiles[0].size.width,
            370.666_666,
            "half of the two-thirds half",
        );
        assert_close(tiles[1].size.width, 370.666_666, "and its sibling");
        assert_close(tiles[2].size.width, 378.666_666, "the remaining third");
        assert_close(tiles[2].origin.x, 197.333_333, "which starts past one gap");
    }

    /// The property that makes the layout usable at all, over every count a selection can
    /// plausibly have.
    #[test]
    fn tiles_never_overlap_and_stay_inside_the_region() {
        let region = region();
        for count in 1..=24 {
            let tiles = tiled(count);
            assert_eq!(tiles.len(), count as usize, "one tile per node");
            for (index, first) in tiles.iter().enumerate() {
                assert!(
                    first.origin.x >= region.origin.x - 0.001
                        && first.origin.y >= region.origin.y - 0.001
                        && first.max().x <= region.max().x + 0.001
                        && first.max().y <= region.max().y + 0.001,
                    "{count} tiles: {first:?} escapes {region:?}"
                );
                for second in &tiles[index + 1..] {
                    assert!(
                        !first.intersects(*second),
                        "{count} tiles: {first:?} overlaps {second:?}"
                    );
                }
            }
        }
    }

    fn document_with(ids: [&str; 2]) -> (Document, Vec<NodeId>) {
        let mut document = Document::load(CanvasDocument::empty());
        let ids: Vec<NodeId> = ids
            .iter()
            .map(|id| NodeId::new((*id).to_string()))
            .collect();
        for id in &ids {
            document.insert_node(
                id.clone(),
                NodeType::Text,
                Rect::new(Point::new(0.0, 0.0), Size::new(280.0, 140.0)),
            );
        }
        document.select_all();
        (document, ids)
    }

    /// Tiles go to nodes in page order, the way `rf.getNodes()` hands them over — not in the
    /// sorted order `Document::selected` keeps them in. The ids here sort the other way round,
    /// so a selection-ordered implementation swaps the two nodes and fails.
    #[test]
    fn tiles_follow_page_order_not_selection_order() {
        let (mut document, ids) = document_with(["z", "a"]);
        assert!(fit_selection(&mut document, PANE, Point::new(0.0, 0.0)));

        let left = document.node(&ids[0]).expect("still there").position.x;
        let right = document.node(&ids[1]).expect("still there").position.x;
        assert!(
            left < right,
            "the first node on the page took the first tile: {left} then {right}"
        );
    }

    #[test]
    fn an_empty_selection_writes_nothing() {
        let (mut document, _) = document_with(["a", "b"]);
        document.select_only([]);
        assert!(!fit_selection(&mut document, PANE, Point::new(0.0, 0.0)));
    }

    /// Every node moved and resized, and one press puts all of it back.
    #[test]
    fn a_fit_is_one_undo_step() {
        let (mut document, ids) = document_with(["a", "b"]);
        let before: Vec<Rect> = ids
            .iter()
            .map(|id| document.node(id).expect("still there").bounds())
            .collect();
        document.checkpoint();

        assert!(fit_selection(&mut document, PANE, Point::new(0.0, 0.0)));
        document.checkpoint();
        let after: Vec<Rect> = ids
            .iter()
            .map(|id| document.node(id).expect("still there").bounds())
            .collect();
        assert_ne!(before, after, "the fit wrote positions and sizes");

        assert!(document.undo(), "the fit is undoable");
        let restored: Vec<Rect> = ids
            .iter()
            .map(|id| document.node(id).expect("still there").bounds())
            .collect();
        assert_eq!(before, restored, "one press restores both nodes entirely");
    }
}
