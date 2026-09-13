//! Island anchors: the port of `~/labs/peek/src/canvas/layout/componentAnchors.ts`.
//!
//! Every node is pulled toward one anchor per connected component, and the components are
//! shelf-packed largest-first into a roughly square grid centred on the origin. Without it a
//! disconnected subgraph has nothing to hold it and the charge force pushes it to infinity.
//!
//! The reference also clusters by *region*, one island per region regardless of edges. Regions
//! have a mutation API here but no renderer yet, so that branch is deliberately not ported —
//! with no region map the reference's own `computeClusters` reduces to connected components,
//! which is exactly what this is.

use peek_document::geometry::Point;

const AREA_FACTOR: f64 = 2.5;
const ISLAND_GAP: f64 = 250.0;

/// All the packer needs of a node: its size.
pub(super) struct Extent {
    pub(super) width: f64,
    pub(super) height: f64,
}

fn connected_components(bodies: &[Extent], links: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); bodies.len()];
    for &(source, target) in links {
        adjacency[source].push(target);
        adjacency[target].push(source);
    }

    let mut visited = vec![false; bodies.len()];
    let mut components = Vec::new();
    for root in 0..bodies.len() {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let index = queue[head];
            head += 1;
            for &neighbour in &adjacency[index] {
                if !visited[neighbour] {
                    visited[neighbour] = true;
                    queue.push(neighbour);
                }
            }
        }
        components.push(queue);
    }
    components
}

fn component_radius(bodies: &[Extent], component: &[usize]) -> f64 {
    if let [only] = component {
        return bodies[*only].width.max(bodies[*only].height) / 2.0;
    }
    let area: f64 = component
        .iter()
        .map(|&index| bodies[index].width * bodies[index].height)
        .sum();
    (area * AREA_FACTOR / std::f64::consts::PI).sqrt()
}

/// One anchor per node, indexed the same way `bodies` is.
pub(super) fn compute(bodies: &[Extent], links: &[(usize, usize)]) -> Vec<Point> {
    let mut islands: Vec<(Vec<usize>, f64)> = connected_components(bodies, links)
        .into_iter()
        .map(|component| {
            let cell = component_radius(bodies, &component).mul_add(2.0, ISLAND_GAP);
            (component, cell)
        })
        .collect();
    // Largest first, and by first member on a tie so the packing does not depend on the sort's
    // stability guarantees the way `toSorted` lets the reference depend on them.
    islands.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0[0].cmp(&right.0[0]))
    });

    let diagonal: f64 = islands
        .iter()
        .map(|(_, cell)| cell * cell)
        .sum::<f64>()
        .sqrt();
    let target_row_width = islands.first().map_or(0.0, |(_, cell)| *cell).max(diagonal);

    let mut placements: Vec<(&[usize], Point)> = Vec::with_capacity(islands.len());
    let mut cursor_x = 0.0_f64;
    let mut row_y = 0.0_f64;
    let mut row_height = 0.0_f64;
    let mut max_x = 0.0_f64;
    for (component, cell) in &islands {
        if cursor_x > 0.0 && cursor_x + cell > target_row_width {
            row_y += row_height;
            cursor_x = 0.0;
            row_height = 0.0;
        }
        placements.push((
            component,
            Point::new(cursor_x + cell / 2.0, row_y + cell / 2.0),
        ));
        cursor_x += cell;
        row_height = row_height.max(*cell);
        max_x = max_x.max(cursor_x);
    }
    let max_y = row_y + row_height;

    let mut anchors = vec![Point::default(); bodies.len()];
    for (component, centre) in placements {
        let anchor = Point::new(centre.x - max_x / 2.0, centre.y - max_y / 2.0);
        for &index in component {
            anchors[index] = anchor;
        }
    }
    anchors
}

#[cfg(test)]
mod tests {
    use super::{Extent, compute};

    fn square() -> Extent {
        Extent {
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn one_component_anchors_every_member_on_the_origin() {
        let bodies = [square(), square(), square()];
        let anchors = compute(&bodies, &[(0, 1), (1, 2)]);
        assert_eq!(anchors[0], anchors[1]);
        assert_eq!(anchors[1], anchors[2]);
        assert!(anchors[0].x.abs() < 1e-9 && anchors[0].y.abs() < 1e-9);
    }

    #[test]
    fn disconnected_components_get_their_own_islands() {
        let bodies = [square(), square(), square(), square()];
        let anchors = compute(&bodies, &[(0, 1), (2, 3)]);
        assert_eq!(anchors[0], anchors[1], "a component shares one anchor");
        assert_eq!(anchors[2], anchors[3]);
        assert_ne!(anchors[0], anchors[2], "two components, two islands");
    }
}
