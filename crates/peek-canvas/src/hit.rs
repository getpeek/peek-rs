//! World-space hit tests. Paint order is array order, so "topmost" is the last match.
//!
//! Node regions are resolved here, not by listeners on the node element: `CanvasElement`
//! registers its window-level mouse listeners last and gpui dispatches the bubble phase in
//! reverse registration order, so the canvas always hears a press before the node under it
//! does. Deciding the region from the pointer's world position instead keeps the whole rule
//! in one pure, window-free function.

use peek_document::geometry::{Point, Rect};
use peek_document::{Edge, EdgeId, Node, NodeId, NodeKind, Page};

use crate::edge::{EdgeCurve, curve_between};

/// Height of `NodeShell`'s header in world units: `rems(2.0)` against the 16 px base rem.
/// The shell is laid out inside a rem scope of `base_rem * zoom`, so the header covers the
/// same world band at every zoom level.
pub const HEADER_WORLD_HEIGHT: f64 = 32.0;

/// How far inside an edge still counts as a resize grab, in **screen** pixels: the band is
/// converted to world units by the camera's zoom, so the grab stays the same width under the
/// pointer however far the camera is zoomed out.
pub const RESIZE_EDGE_SCREEN_INSET: f64 = 12.0;

/// The same for a corner, which owns two axes at once and is the harder target to catch.
pub const RESIZE_CORNER_SCREEN_INSET: f64 = 18.0;

/// How far the band may grow past its screen inset as the camera zooms out. Without a cap a
/// card seen at zoom 0.1 would be resize band from edge to edge, leaving nothing to drag.
const MAX_ZOOM_OUT_GROWTH: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Corner {
    /// Which edges this grab moves: `(left, top, right, bottom)`.
    #[must_use]
    fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Self::TopLeft => (true, true, false, false),
            Self::Top => (false, true, false, false),
            Self::TopRight => (false, true, true, false),
            Self::Right => (false, false, true, false),
            Self::BottomRight => (false, false, true, true),
            Self::Bottom => (false, false, false, true),
            Self::BottomLeft => (true, false, false, true),
            Self::Left => (true, false, false, false),
        }
    }

    /// The node's new bounds when this grab is dragged by `delta` world units.
    #[must_use]
    pub fn resize(self, start: Rect, delta: Point) -> Rect {
        let (left, top, right, bottom) = self.edges();
        let mut min = start.min();
        let mut max = start.max();
        if left {
            min.x += delta.x;
        }
        if right {
            max.x += delta.x;
        }
        if top {
            min.y += delta.y;
        }
        if bottom {
            max.y += delta.y;
        }
        Rect::from_corners(min, max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRegion {
    /// The drag handle: press-and-move moves the node, press-and-release selects it.
    Header,
    /// Interactive content. Selects on click, but never starts a node drag or a marquee —
    /// the node's own body owns the pointer from there.
    Body,
    Resize(Corner),
}

/// How far from an edge's curve still counts as a press, in world units: React Flow paints a
/// transparent `interactionWidth` of 20 beside the visible stroke. That stroke lives inside the
/// transformed viewport, so the tolerance is world-space and grows on screen with the zoom.
pub const EDGE_HIT_WORLD_WIDTH: f64 = 20.0;

/// What the pointer is over. Nodes always win: the edge layer is painted beneath the cards.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Node(NodeHit),
    Edge(EdgeId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeHit {
    pub id: NodeId,
    pub region: NodeRegion,
    /// The node's bounds at press time, so a resize is computed from where it started.
    pub bounds: Rect,
}

/// What `world` lands on, nodes before edges.
#[must_use]
pub fn hit_at(page: &Page, world: Point, zoom: f64) -> Option<Hit> {
    node_hit_at(&page.nodes, world, zoom)
        .map(Hit::Node)
        .or_else(|| edge_hit_at(page, world).map(Hit::Edge))
}

/// The topmost edge whose curve passes within half of [`EDGE_HIT_WORLD_WIDTH`] of `world`.
/// An edge naming a node that is not on the page has no curve and is skipped.
#[must_use]
pub fn edge_hit_at(page: &Page, world: Point) -> Option<EdgeId> {
    let tolerance = EDGE_HIT_WORLD_WIDTH / 2.0;
    page.edges
        .iter()
        .rev()
        .find(|edge| {
            curve_of(page, edge).is_some_and(|curve| {
                curve.bounds().dilated(tolerance).contains(world)
                    && curve.distance_to(world) <= tolerance
            })
        })
        .map(|edge| edge.id.clone())
}

fn curve_of(page: &Page, edge: &Edge) -> Option<EdgeCurve> {
    let source = page.node(&edge.source)?.bounds();
    let target = page.node(&edge.target)?.bounds();
    Some(curve_between(source, target))
}

/// Nodes whose bounds overlap `rect` at all (React Flow `SelectionMode.Partial`).
pub fn nodes_in_rect(nodes: &[Node], rect: Rect) -> impl Iterator<Item = &NodeId> {
    nodes
        .iter()
        .filter(move |node| node.bounds().intersects(rect))
        .map(|node| &node.id)
}

#[must_use]
pub fn topmost_node_at(nodes: &[Node], world: Point) -> Option<&NodeId> {
    nodes
        .iter()
        .rev()
        .find(|node| node.bounds().contains(world))
        .map(|node| &node.id)
}

/// Whether presses pass straight through this kind to the canvas beneath.
///
/// Draw nodes are committed with `pointerEvents: "none"` by `useDrawTool.ts`: a stroke is
/// decoration, not a card, so it is never a drag handle and has no resize corners. It can
/// still be marquee-selected, which is why [`nodes_in_rect`] does not filter it.
fn passes_pointers_through(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Draw(_))
}

/// The topmost node under `world` and which of its regions was hit.
#[must_use]
pub fn node_hit_at(nodes: &[Node], world: Point, zoom: f64) -> Option<NodeHit> {
    let node = nodes
        .iter()
        .rev()
        .filter(|node| !passes_pointers_through(node))
        .find(|node| node.bounds().contains(world))?;
    let bounds = node.bounds();
    Some(NodeHit {
        id: node.id.clone(),
        region: region_of(bounds, world, zoom),
        bounds,
    })
}

/// A screen-space grab band in world units at `zoom`, capped by [`MAX_ZOOM_OUT_GROWTH`].
fn world_inset(screen_inset: f64, zoom: f64) -> f64 {
    (screen_inset / zoom).min(screen_inset * MAX_ZOOM_OUT_GROWTH)
}

/// Which edges `world` is within `inset` world units of: `(left, top, right, bottom)`.
/// Never let the band swallow a small node: at most a third of each axis.
fn near_edges(bounds: Rect, world: Point, inset: f64) -> (bool, bool, bool, bool) {
    let (min, max) = (bounds.min(), bounds.max());
    let inset_x = inset.min(bounds.size.width / 3.0);
    let inset_y = inset.min(bounds.size.height / 3.0);
    (
        world.x - min.x <= inset_x,
        world.y - min.y <= inset_y,
        max.x - world.x <= inset_x,
        max.y - world.y <= inset_y,
    )
}

/// Resize zones win over the header, so the top corners stay grabbable. Corners claim a wider
/// band than the sides do, because they are the harder target and sit inside them.
fn region_of(bounds: Rect, world: Point, zoom: f64) -> NodeRegion {
    let corner = match near_edges(bounds, world, world_inset(RESIZE_CORNER_SCREEN_INSET, zoom)) {
        (true, true, ..) => Some(Corner::TopLeft),
        (_, true, true, _) => Some(Corner::TopRight),
        (_, _, true, true) => Some(Corner::BottomRight),
        (true, _, _, true) => Some(Corner::BottomLeft),
        _ => None,
    };
    let side = match near_edges(bounds, world, world_inset(RESIZE_EDGE_SCREEN_INSET, zoom)) {
        (true, ..) => Some(Corner::Left),
        (_, true, ..) => Some(Corner::Top),
        (_, _, true, _) => Some(Corner::Right),
        (_, _, _, true) => Some(Corner::Bottom),
        _ => None,
    };
    match corner.or(side) {
        Some(corner) => NodeRegion::Resize(corner),
        None if world.y - bounds.min().y <= HEADER_WORLD_HEIGHT => NodeRegion::Header,
        None => NodeRegion::Body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use peek_document::geometry::Size;
    use peek_document::{DrawData, NodeType, TextData};

    fn nodes() -> Vec<Node> {
        vec![Node {
            id: NodeId::from("t1"),
            position: Point::new(0.0, 0.0),
            width: Some(300.0),
            height: Some(200.0),
            measured: None,
            selected: false,
            kind: NodeKind::Text(TextData::default()),
        }]
    }

    fn region(x: f64, y: f64) -> NodeRegion {
        region_at_zoom(x, y, 1.0)
    }

    fn region_at_zoom(x: f64, y: f64, zoom: f64) -> NodeRegion {
        node_hit_at(&nodes(), Point::new(x, y), zoom)
            .unwrap()
            .region
    }

    #[test]
    fn header_body_and_resize_bands() {
        assert_eq!(region(150.0, 20.0), NodeRegion::Header);
        assert_eq!(region(150.0, 100.0), NodeRegion::Body);
        assert_eq!(region(150.0, 199.0), NodeRegion::Resize(Corner::Bottom));
        assert_eq!(region(2.0, 2.0), NodeRegion::Resize(Corner::TopLeft));
        assert_eq!(
            region(298.0, 198.0),
            NodeRegion::Resize(Corner::BottomRight)
        );
        assert_eq!(region(2.0, 100.0), NodeRegion::Resize(Corner::Left));
        assert!(node_hit_at(&nodes(), Point::new(400.0, 400.0), 1.0).is_none());
    }

    #[test]
    fn the_grab_band_keeps_its_screen_width_as_the_camera_zooms_out() {
        // 20 world units from the bottom edge: outside the 12-unit band at zoom 1, inside the
        // 24-unit one at zoom 0.5 — the same 12 screen pixels under the pointer either way.
        assert_eq!(region_at_zoom(150.0, 180.0, 1.0), NodeRegion::Body);
        assert_eq!(
            region_at_zoom(150.0, 180.0, 0.5),
            NodeRegion::Resize(Corner::Bottom)
        );
        // And it stops growing there, so a card zoomed far out still has a body to drag.
        assert_eq!(region_at_zoom(150.0, 100.0, 0.1), NodeRegion::Body);
    }

    /// A corner claims a wider band than the side it sits in.
    #[test]
    fn a_corner_wins_over_the_side_that_shares_its_band() {
        assert_eq!(region(15.0, 15.0), NodeRegion::Resize(Corner::TopLeft));
        assert_eq!(region(15.0, 100.0), NodeRegion::Body);
    }

    #[test]
    fn each_corner_moves_the_edges_it_owns() {
        let start = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let delta = Point::new(10.0, 10.0);

        assert_eq!(
            Corner::BottomRight.resize(start, delta),
            Rect::new(Point::new(0.0, 0.0), Size::new(110.0, 110.0))
        );
        assert_eq!(
            Corner::TopLeft.resize(start, delta),
            Rect::new(Point::new(10.0, 10.0), Size::new(90.0, 90.0))
        );
        assert_eq!(
            Corner::Right.resize(start, delta),
            Rect::new(Point::new(0.0, 0.0), Size::new(110.0, 100.0))
        );
        assert_eq!(
            Corner::Top.resize(start, delta),
            Rect::new(Point::new(0.0, 10.0), Size::new(100.0, 90.0))
        );
    }

    #[test]
    fn a_node_shorter_than_the_header_is_never_all_body() {
        // Text's minimum is 80x32 — exactly the header band. Every point in such a node is a
        // resize grab or header, so a tiny node can still be grabbed and moved.
        let tiny = vec![Node::new(
            NodeType::Text,
            Rect::new(Point::new(0.0, 0.0), Size::new(1.0, 1.0)),
        )];
        let size = tiny[0].size();
        for x in [0.5, size.width / 2.0, size.width - 0.5] {
            for y in [0.5, size.height / 2.0, size.height - 0.5] {
                let region = node_hit_at(&tiny, Point::new(x, y), 1.0).unwrap().region;
                assert_ne!(region, NodeRegion::Body, "at ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_drawing_passes_presses_through_to_the_canvas() {
        // `useDrawTool.ts` commits strokes with `pointerEvents: "none"`: a drawing is not a
        // card, so it has no drag handle and no resize corners.
        let drawing = vec![Node {
            id: NodeId::from("draw_1"),
            position: Point::new(0.0, 0.0),
            width: Some(300.0),
            height: Some(200.0),
            measured: None,
            selected: false,
            kind: NodeKind::Draw(DrawData {
                points: Vec::new(),
                stroke_width: 4.0,
                color: "var(--pk-fg)".to_string(),
            }),
        }];

        assert!(
            node_hit_at(&drawing, Point::new(150.0, 10.0), 1.0).is_none(),
            "no header"
        );
        assert!(
            node_hit_at(&drawing, Point::new(298.0, 198.0), 1.0).is_none(),
            "no resize corner"
        );
        assert!(
            node_hit_at(&drawing, Point::new(150.0, 100.0), 1.0).is_none(),
            "no body"
        );

        // It is still marquee-selectable: pointer transparency is not invisibility.
        let rect = Rect::new(Point::new(-10.0, -10.0), Size::new(400.0, 400.0));
        assert_eq!(nodes_in_rect(&drawing, rect).count(), 1);
    }

    /// A node painted over a drawing still receives presses.
    #[test]
    fn a_drawing_does_not_shadow_the_node_beneath_it() {
        let mut both = nodes();
        both.push(Node {
            id: NodeId::from("draw_1"),
            position: Point::new(0.0, 0.0),
            width: Some(300.0),
            height: Some(200.0),
            measured: None,
            selected: false,
            kind: NodeKind::Draw(DrawData {
                points: Vec::new(),
                stroke_width: 4.0,
                color: "white".to_string(),
            }),
        });

        let hit = node_hit_at(&both, Point::new(150.0, 100.0), 1.0).expect("the text node is hit");
        assert_eq!(hit.id, NodeId::from("t1"));
    }

    /// A page with two 100x100 nodes on the same row, wired source -> target. Their curve is
    /// the straight line `y = 50` running from x = 100 to x = 300.
    fn wired_page() -> Page {
        let mut page = Page::new("p");
        for (id, x) in [("source", 0.0), ("target", 300.0)] {
            page.nodes.push(Node {
                id: NodeId::from(id),
                position: Point::new(x, 0.0),
                width: Some(100.0),
                height: Some(100.0),
                measured: None,
                selected: false,
                kind: NodeKind::Text(TextData::default()),
            });
        }
        page.edges.push(Edge::between(
            NodeId::from("source"),
            NodeId::from("target"),
        ));
        page
    }

    #[test]
    fn a_press_within_the_interaction_width_hits_the_edge() {
        let page = wired_page();
        let tolerance = EDGE_HIT_WORLD_WIDTH / 2.0;

        assert_eq!(
            edge_hit_at(&page, Point::new(200.0, 50.0 + tolerance - 0.5)),
            Some(EdgeId::from("source->target")),
            "just inside the interaction width"
        );
        assert!(
            edge_hit_at(&page, Point::new(200.0, 50.0 + tolerance + 0.5)).is_none(),
            "and just outside it"
        );
    }

    #[test]
    fn a_node_wins_over_an_edge_running_under_it() {
        let mut page = wired_page();
        // A card straddling the middle of the curve, painted after both endpoints.
        page.nodes.push(Node {
            id: NodeId::from("over"),
            position: Point::new(150.0, 0.0),
            width: Some(100.0),
            height: Some(100.0),
            measured: None,
            selected: false,
            kind: NodeKind::Text(TextData::default()),
        });

        assert_eq!(
            hit_at(&page, Point::new(200.0, 50.0), 1.0),
            node_hit_at(&page.nodes, Point::new(200.0, 50.0), 1.0).map(Hit::Node),
            "the card takes the press, not the edge beneath it"
        );
    }

    #[test]
    fn an_edge_under_a_drawing_is_still_pressable() {
        let mut page = wired_page();
        page.nodes.push(Node {
            id: NodeId::from("draw_1"),
            position: Point::new(150.0, 0.0),
            width: Some(100.0),
            height: Some(100.0),
            measured: None,
            selected: false,
            kind: NodeKind::Draw(DrawData {
                points: Vec::new(),
                stroke_width: 4.0,
                color: "white".to_string(),
            }),
        });

        assert_eq!(
            hit_at(&page, Point::new(200.0, 50.0), 1.0),
            Some(Hit::Edge(EdgeId::from("source->target"))),
            "a drawing passes presses through to the edge, as it does to a node"
        );
    }

    #[test]
    fn an_edge_with_a_missing_endpoint_is_not_pressable() {
        let mut page = wired_page();
        page.nodes.retain(|node| node.id != NodeId::from("target"));

        assert!(
            edge_hit_at(&page, Point::new(200.0, 50.0)).is_none(),
            "a dangling edge has no curve to press, and must not panic"
        );
    }

    #[test]
    fn the_last_edge_in_the_page_wins_when_two_overlap() {
        let mut page = wired_page();
        // The reverse pairing traces the same line, and is painted last.
        page.edges.push(Edge::between(
            NodeId::from("target"),
            NodeId::from("source"),
        ));

        assert_eq!(
            edge_hit_at(&page, Point::new(200.0, 50.0)),
            Some(EdgeId::from("target->source")),
            "paint order is array order, so the topmost match is the last one"
        );
    }
}
