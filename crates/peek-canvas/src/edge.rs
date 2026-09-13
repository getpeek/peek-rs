//! The floating bezier an edge is drawn as: a port of
//! `~/labs/peek/src/canvas/edges/floatingEdgeUtils.ts` plus React Flow's `getBezierPath`.
//!
//! Peek's edges carry no handles. Both endpoints are recomputed from the live node rectangles
//! every frame: each one is where the line between the two node centres crosses that node's
//! box, so the curve stays well-routed however the nodes are dragged around. The side it
//! crosses also decides which way the curve leaves, which is why [`Side`] exists at all.

use peek_document::geometry::{Point, Rect};

/// React Flow's default `getBezierPath` curvature.
const CURVATURE: f64 = 0.25;

/// How finely [`EdgeCurve::distance_to`] flattens the curve. Twenty-four chords hold the error
/// well under the hit tolerance at the curve lengths node spacing produces.
const HIT_SAMPLES: u32 = 24;

/// One cubic bezier in world units: `M start C control_start control_end end`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeCurve {
    pub start: Point,
    pub control_start: Point,
    pub control_end: Point,
    pub end: Point,
}

/// Which side of a node the curve leaves through (`getEdgePosition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// The floating bezier running from `source` to `target`, both in world units.
#[must_use]
pub fn curve_between(source: Rect, target: Rect) -> EdgeCurve {
    let start = intersection(source, target);
    let end = intersection(target, source);
    EdgeCurve {
        start,
        control_start: control(side_of(source, start), start, end),
        control_end: control(side_of(target, end), end, start),
        end,
    }
}

impl EdgeCurve {
    /// The control hull. A bezier never leaves the hull of its control points, so this is the
    /// cheap reject box for culling and hit-testing.
    #[must_use]
    pub fn bounds(self) -> Rect {
        let points = [self.start, self.control_start, self.control_end, self.end];
        let min = points.iter().fold(points[0], |min, point| {
            Point::new(min.x.min(point.x), min.y.min(point.y))
        });
        let max = points.iter().fold(points[0], |max, point| {
            Point::new(max.x.max(point.x), max.y.max(point.y))
        });
        Rect::from_corners(min, max)
    }

    /// How far `world` is from the curve, flattened to [`HIT_SAMPLES`] chords.
    #[must_use]
    pub fn distance_to(self, world: Point) -> f64 {
        let mut previous = self.start;
        let mut nearest = f64::INFINITY;
        for step in 1..=HIT_SAMPLES {
            let next = self.sample(f64::from(step) / f64::from(HIT_SAMPLES));
            nearest = nearest.min(distance_to_segment(world, previous, next));
            previous = next;
        }
        nearest
    }

    /// The point at `along` down the curve, `along` in `0..=1`: the Bernstein weights of a
    /// cubic bezier applied to the four control points.
    fn sample(self, along: f64) -> Point {
        let rest = 1.0 - along;
        let start = rest * rest * rest;
        let control_start = 3.0 * rest * rest * along;
        let control_end = 3.0 * rest * along * along;
        let end = along * along * along;
        Point::new(
            start * self.start.x
                + control_start * self.control_start.x
                + control_end * self.control_end.x
                + end * self.end.x,
            start * self.start.y
                + control_start * self.control_start.y
                + control_end * self.control_end.y
                + end * self.end.y,
        )
    }
}

/// Where the line between the two centres crosses `rect`'s box, the trick the official xyflow
/// floating-edges example uses. A node with no extent has no border to land on, so it keeps
/// its centre.
fn intersection(rect: Rect, toward: Rect) -> Point {
    let (half_width, half_height) = (rect.size.width / 2.0, rect.size.height / 2.0);
    let centre = rect.center();
    if half_width <= 0.0 || half_height <= 0.0 {
        return centre;
    }
    let other = toward.center();
    let (dx, dy) = (other.x - centre.x, other.y - centre.y);
    let first = dx / (2.0 * half_width) - dy / (2.0 * half_height);
    let second = dx / (2.0 * half_width) + dy / (2.0 * half_height);
    // `a1 = 1 / (|x1| + |x2| || 1)`: concentric centres would divide by zero.
    let spread = first.abs() + second.abs();
    let scale = if spread == 0.0 { 1.0 } else { 1.0 / spread };
    let (x, y) = (scale * first, scale * second);
    Point::new(
        half_width * (x + y) + centre.x,
        half_height * (y - x) + centre.y,
    )
}

/// The reference rounds both the point and the origin, but not the extent, before comparing.
fn side_of(rect: Rect, point: Point) -> Side {
    let min = rect.min();
    let (x, y) = (point.x.round(), point.y.round());
    let (left, top) = (min.x.round(), min.y.round());
    if x <= left + 1.0 {
        return Side::Left;
    }
    if x >= left + rect.size.width - 1.0 {
        return Side::Right;
    }
    if y <= top + 1.0 {
        return Side::Top;
    }
    if y >= top + rect.size.height - 1.0 {
        return Side::Bottom;
    }
    Side::Top
}

/// Nodes far apart get a control point half the gap away; overlapping ones, where the gap is
/// negative, fall back to a square-root splay so the curve still bulges instead of collapsing.
fn control_offset(distance: f64) -> f64 {
    if distance >= 0.0 {
        return 0.5 * distance;
    }
    CURVATURE * 25.0 * (-distance).sqrt()
}

/// The control point for an endpoint, pushed straight out of the side it leaves through.
fn control(side: Side, from: Point, toward: Point) -> Point {
    match side {
        Side::Left => Point::new(from.x - control_offset(from.x - toward.x), from.y),
        Side::Right => Point::new(from.x + control_offset(toward.x - from.x), from.y),
        Side::Top => Point::new(from.x, from.y - control_offset(from.y - toward.y)),
        Side::Bottom => Point::new(from.x, from.y + control_offset(toward.y - from.y)),
    }
}

fn distance_to_segment(point: Point, from: Point, to: Point) -> f64 {
    let span = to - from;
    let length_squared = span.x * span.x + span.y * span.y;
    if length_squared <= f64::EPSILON {
        return point.distance_to(from);
    }
    let offset = point - from;
    let along = ((offset.x * span.x + offset.y * span.y) / length_squared).clamp(0.0, 1.0);
    point.distance_to(from + span.scaled(along))
}

#[cfg(test)]
mod tests {
    use peek_document::geometry::{Point, Rect, Size};

    use super::{EdgeCurve, control_offset, curve_between};

    fn rect(x: f64, y: f64) -> Rect {
        Rect::new(Point::new(x, y), Size::new(100.0, 100.0))
    }

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    #[test]
    fn endpoints_land_on_the_facing_borders_not_the_centres() {
        let curve = curve_between(rect(0.0, 0.0), rect(300.0, 0.0));

        assert_eq!(
            curve.start,
            Point::new(100.0, 50.0),
            "the source leaves through its right border"
        );
        assert_eq!(
            curve.end,
            Point::new(300.0, 50.0),
            "the target is entered through its left border"
        );
    }

    #[test]
    fn a_stacked_pair_leaves_through_the_horizontal_borders() {
        let curve = curve_between(rect(0.0, 0.0), rect(0.0, 300.0));

        assert_eq!(curve.start, Point::new(50.0, 100.0), "bottom of the source");
        assert_eq!(curve.end, Point::new(50.0, 300.0), "top of the target");
        assert!(
            close(curve.control_start.x, curve.start.x) && curve.control_start.y > curve.start.y,
            "a curve leaving the bottom is pushed straight down: {:?}",
            curve.control_start
        );
        assert!(
            close(curve.control_end.x, curve.end.x) && curve.control_end.y < curve.end.y,
            "and enters the target from straight above: {:?}",
            curve.control_end
        );
    }

    #[test]
    fn a_node_with_no_extent_keeps_its_centre() {
        let collapsed = Rect::new(Point::new(10.0, 10.0), Size::new(0.0, 0.0));

        let curve = curve_between(collapsed, rect(300.0, 0.0));

        assert_eq!(
            curve.start,
            Point::new(10.0, 10.0),
            "there is no border to land on, so the centre stands in"
        );
    }

    #[test]
    fn overlapping_nodes_splay_the_controls_instead_of_collapsing_them() {
        // The gap between the facing borders is negative, so the halved-distance branch would
        // pull each control back behind its own endpoint and flatten the curve to nothing.
        let curve = curve_between(rect(0.0, 0.0), rect(20.0, 0.0));

        assert!(
            curve.control_start.x > curve.start.x,
            "the control still bulges away from the source: {:?}",
            curve.control_start
        );
        assert!(
            close(curve.control_start.x - curve.start.x, control_offset(-80.0)),
            "by the square-root splay, not half the (negative) gap"
        );
    }

    #[test]
    fn the_hull_contains_both_endpoints() {
        let curve = curve_between(rect(0.0, 0.0), rect(300.0, 200.0));
        let bounds = curve.bounds();

        assert!(bounds.contains(curve.start) && bounds.contains(curve.end));
    }

    #[test]
    fn distance_is_zero_on_the_curve_and_grows_away_from_it() {
        let curve = curve_between(rect(0.0, 0.0), rect(300.0, 0.0));
        // Both nodes sit on the same row, so the whole curve is the straight line y = 50.
        let on_curve = Point::new(200.0, 50.0);

        assert!(
            curve.distance_to(on_curve) < 1e-6,
            "a point on the curve: {}",
            curve.distance_to(on_curve)
        );
        assert!(
            close(curve.distance_to(Point::new(200.0, 80.0)), 30.0),
            "and one thirty units above it: {}",
            curve.distance_to(Point::new(200.0, 80.0))
        );
    }

    #[test]
    fn sampling_runs_from_start_to_end() {
        let curve: EdgeCurve = curve_between(rect(0.0, 0.0), rect(300.0, 200.0));

        assert_eq!(curve.sample(0.0), curve.start);
        assert_eq!(curve.sample(1.0), curve.end);
    }
}
