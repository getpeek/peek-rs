//! Walking the selection from node to node with the arrow keys. The port of `pickInDirection`
//! and `nearestToViewportCenter` in `src/canvas/hooks/usePageActions.ts`.

use peek_document::{Node, NodeId};

use crate::Point;

/// Off-axis drift costs double, so among the nodes inside the cone the one most squarely in
/// the pressed direction wins even when a skewed node sits nearer.
const PERPENDICULAR_PENALTY: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// How far the offset travels the way the key points, and how far it strays off that axis.
    /// A negative reach is behind the origin.
    fn reach_and_drift(self, offset: Point) -> (f64, f64) {
        match self {
            Self::Up => (-offset.y, offset.x.abs()),
            Self::Down => (offset.y, offset.x.abs()),
            Self::Left => (-offset.x, offset.y.abs()),
            Self::Right => (offset.x, offset.y.abs()),
        }
    }
}

/// The node to move to from `origin`, or `None` when nothing lies that way.
///
/// Candidates must sit inside a 45° cone ahead of the origin's centre — exactly 45° counts —
/// and the cheapest weighted distance wins. Ties go to whichever node comes first in the
/// document, which is what the reference's fold over React Flow's node array does.
#[must_use]
pub fn node_in_direction<'a>(
    nodes: &'a [Node],
    origin: &Node,
    direction: Direction,
) -> Option<&'a NodeId> {
    let from = origin.bounds().center();
    let mut best: Option<(&NodeId, f64)> = None;
    for node in nodes {
        if node.id == origin.id {
            continue;
        }
        let (reach, drift) = direction.reach_and_drift(node.bounds().center() - from);
        if reach <= 0.0 || drift > reach {
            continue;
        }
        let score = drift.mul_add(PERPENDICULAR_PENALTY, reach);
        if best.is_none_or(|(_, previous)| score < previous) {
            best = Some((&node.id, score));
        }
    }
    best.map(|(id, _)| id)
}

/// The node whose centre is closest to `point`. What an arrow key anchors on when nothing is
/// selected yet, so the first press starts from whatever the camera is looking at.
#[must_use]
pub fn nearest_to(nodes: &[Node], point: Point) -> Option<&NodeId> {
    nodes
        .iter()
        .min_by(|a, b| {
            let distance = |node: &Node| {
                let offset = node.bounds().center() - point;
                offset.x.mul_add(offset.x, offset.y * offset.y)
            };
            distance(a).total_cmp(&distance(b))
        })
        .map(|node| &node.id)
}

#[cfg(test)]
mod tests {
    use peek_document::NodeType;

    use super::*;
    use crate::{Rect, Size};

    /// A 100×100 node whose *centre* sits at `(x, y)`, so the tests read as geometry rather
    /// than as arithmetic about corners.
    fn centred_at(x: f64, y: f64) -> Node {
        let mut node = Node::new(
            NodeType::Text,
            Rect::new(Point::default(), Size::new(100.0, 100.0)),
        );
        let size = node.size();
        node.position = Point::new(x - size.width / 2.0, y - size.height / 2.0);
        node
    }

    #[test]
    fn picks_the_nearest_node_in_the_pressed_direction() {
        let nodes = vec![
            centred_at(0.0, 0.0),
            centred_at(500.0, 0.0),
            centred_at(200.0, 0.0),
        ];
        let picked = node_in_direction(&nodes, &nodes[0], Direction::Right);
        assert_eq!(picked, Some(&nodes[2].id));
    }

    #[test]
    fn nodes_behind_and_beside_the_origin_are_rejected() {
        let nodes = vec![
            centred_at(0.0, 0.0),
            centred_at(-200.0, 0.0),
            centred_at(0.0, 300.0),
        ];
        assert_eq!(node_in_direction(&nodes, &nodes[0], Direction::Right), None);
        assert_eq!(
            node_in_direction(&nodes, &nodes[0], Direction::Left),
            Some(&nodes[1].id)
        );
        assert_eq!(
            node_in_direction(&nodes, &nodes[0], Direction::Down),
            Some(&nodes[2].id)
        );
    }

    #[test]
    fn the_cone_is_45_degrees_and_includes_its_edge() {
        let origin = centred_at(0.0, 0.0);
        let on_the_edge = centred_at(100.0, 100.0);
        let just_outside = centred_at(100.0, 101.0);

        let inside = vec![origin.clone(), on_the_edge];
        assert!(node_in_direction(&inside, &origin, Direction::Right).is_some());

        let outside = vec![origin.clone(), just_outside];
        assert_eq!(node_in_direction(&outside, &origin, Direction::Right), None);
    }

    #[test]
    fn a_squarely_placed_node_beats_a_nearer_skewed_one() {
        let nodes = vec![
            centred_at(0.0, 0.0),
            // Nearer in a straight line, but 45° off: 100 + 99 × 2 = 298.
            centred_at(100.0, 99.0),
            // Further away, but dead ahead: 200 + 0 = 200.
            centred_at(200.0, 0.0),
        ];
        assert_eq!(
            node_in_direction(&nodes, &nodes[0], Direction::Right),
            Some(&nodes[2].id)
        );
    }

    #[test]
    fn a_tie_goes_to_the_earlier_node_in_the_document() {
        let nodes = vec![
            centred_at(0.0, 0.0),
            centred_at(200.0, 50.0),
            centred_at(200.0, -50.0),
        ];
        assert_eq!(
            node_in_direction(&nodes, &nodes[0], Direction::Right),
            Some(&nodes[1].id)
        );
    }

    #[test]
    fn nearest_to_finds_the_closest_centre() {
        let nodes = vec![centred_at(0.0, 0.0), centred_at(90.0, 0.0)];
        assert_eq!(
            nearest_to(&nodes, Point::new(100.0, 0.0)),
            Some(&nodes[1].id)
        );
        assert_eq!(nearest_to(&[], Point::default()), None);
    }
}
