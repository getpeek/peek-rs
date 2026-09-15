//! The force-directed layout behind `View::Organize` and the schema page.
//!
//! A port of d3-force as `~/labs/peek/src/canvas/layout/forceSimulation.ts` configures it —
//! link springs, a repulsive charge, one anchor per connected component and a *rectangular*
//! collision force, in that order, which is the order d3 runs them in. The constants are the
//! reference's; so are the integration rules (`alpha += (target - alpha) * decay`, then each
//! force, then `v *= 0.6; position += v`), because a layout that settles differently is a
//! different layout.
//!
//! Pure maths, so it lives here rather than in the view: a run is deterministic given its
//! starting positions, which is what makes it testable with `cargo test -p peek-canvas`.
//! [`crate::layout::random`] explains why even the jiggle is reproducible.
//!
//! [`bsp`] is the other layout on this page: the one-shot viewport tiling behind
//! `Zoom::FitSelection`, which shares nothing with the simulation but the module.

mod anchors;
mod random;

pub mod bsp;
pub mod schema;

use peek_document::geometry::Point;
use peek_document::{NodeId, PageId};

use crate::history::EditKind;
use crate::model::Document;
use random::Lcg;

/// d3's `alphaMin`: below this the simulation is at rest and stops.
const ALPHA_MIN: f64 = 0.001;
/// d3's `velocityDecay`, applied as a multiplier every tick.
const VELOCITY_DECAY: f64 = 0.6;
/// `ANIMATED_ALPHA_DECAY` in `organizeCanvas.tsx`: three times d3's default, so a run settles in
/// about a second and a half instead of five.
const ALPHA_DECAY: f64 = 0.07;

const LINK_GAP: f64 = 100.0;
const LINK_STRENGTH: f64 = 0.6;
const CHARGE_STRENGTH: f64 = -800.0;
const CHARGE_DISTANCE_MAX_SQUARED: f64 = 1000.0 * 1000.0;
/// d3's `distanceMin`, squared: the floor that stops two near-coincident nodes exploding.
const CHARGE_DISTANCE_MIN_SQUARED: f64 = 1.0;
const ANCHOR_STRENGTH: f64 = 0.2;
const COLLIDE_PADDING: f64 = 48.0;
/// Corrections are scaled by a fixed strength rather than by alpha, unlike d3's own
/// `forceCollide`: an alpha-scaled shove fades as the simulation cools and freezes residual
/// overlaps into the final layout.
const COLLIDE_STRENGTH: f64 = 0.7;
const COLLIDE_ITERATIONS: usize = 3;

/// `FIT_EVERY_TICKS` in `organizeCanvas.tsx`: the camera follows the layout out rather than
/// waiting for it to finish somewhere off-screen.
const FIT_EVERY_TICKS: u64 = 8;

/// Below this the node is treated as not having moved, so a settled simulation stops bumping
/// the document's revision and re-arming the autosave.
const MOVE_EPSILON: f64 = 0.01;

/// One node in the simulation. Coordinates are **centres**; the document stores top-left
/// origins, and [`Layout::apply`] is the only place the two meet.
#[derive(Debug)]
struct Body {
    id: NodeId,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
}

#[derive(Debug, Clone, Copy)]
struct Link {
    source: usize,
    target: usize,
    distance: f64,
    /// d3's `bias`: how the spring's correction splits between the two ends, by degree. A leaf
    /// hanging off a hub moves; the hub barely does.
    bias: f64,
}

/// A running force layout over one page.
#[derive(Debug)]
pub struct Layout {
    page: PageId,
    bodies: Vec<Body>,
    links: Vec<Link>,
    anchors: Vec<Point>,
    alpha: f64,
    ticks: u64,
    random: Lcg,
}

impl Layout {
    /// Seeds a layout from the active page, starting every node where it already is.
    ///
    /// `None` below two nodes: there is nothing to arrange, and the caller just fits the view.
    #[must_use]
    pub fn of_active_page(document: &Document) -> Option<Self> {
        let nodes = document.nodes();
        if nodes.len() < 2 {
            return None;
        }

        let bodies: Vec<Body> = nodes
            .iter()
            .map(|node| {
                let size = node.size();
                let bounds = node.bounds().center();
                Body {
                    id: node.id.clone(),
                    width: size.width,
                    height: size.height,
                    x: bounds.x,
                    y: bounds.y,
                    vx: 0.0,
                    vy: 0.0,
                }
            })
            .collect();

        let index_of = |id: &NodeId| bodies.iter().position(|body| &body.id == id);
        let pairs: Vec<(usize, usize)> = document
            .edges()
            .iter()
            .filter_map(|edge| Some((index_of(&edge.source)?, index_of(&edge.target)?)))
            .collect();

        let mut degree = vec![0_usize; bodies.len()];
        for &(source, target) in &pairs {
            degree[source] += 1;
            degree[target] += 1;
        }

        #[allow(
            clippy::cast_precision_loss,
            reason = "a degree far below 2^53 converts exactly"
        )]
        let links: Vec<Link> = pairs
            .iter()
            .map(|&(source, target)| Link {
                source,
                target,
                distance: f64::midpoint(
                    max_dimension(&bodies[source]),
                    max_dimension(&bodies[target]),
                ) + LINK_GAP,
                bias: degree[source] as f64 / (degree[source] + degree[target]) as f64,
            })
            .collect();

        let extents: Vec<anchors::Extent> = bodies
            .iter()
            .map(|body| anchors::Extent {
                width: body.width,
                height: body.height,
            })
            .collect();

        Some(Self {
            page: document.active_page_id().clone(),
            anchors: anchors::compute(&extents, &pairs),
            bodies,
            links,
            alpha: 1.0,
            ticks: 0,
            random: Lcg::default(),
        })
    }

    /// The page the run started on. Positions are written to whatever page is active, so a
    /// caller that lets the user switch pages mid-run has to stop it.
    #[must_use]
    pub fn page(&self) -> &PageId {
        &self.page
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.alpha < ALPHA_MIN
    }

    /// Advances one d3 tick. Returns whether the camera should re-fit on this one.
    pub fn step(&mut self) -> bool {
        self.alpha -= self.alpha * ALPHA_DECAY;
        self.link_force();
        self.charge_force();
        self.anchor_force();
        self.collide_force();
        for body in &mut self.bodies {
            body.vx *= VELOCITY_DECAY;
            body.x += body.vx;
            body.vy *= VELOCITY_DECAY;
            body.y += body.vy;
        }
        self.ticks += 1;
        self.ticks.is_multiple_of(FIT_EVERY_TICKS)
    }

    /// Writes the current positions back. Returns whether anything actually moved.
    ///
    /// One [`EditKind::Move`] transaction per call, which is what folds a whole run into a
    /// single undo step: consecutive `Move` edits inside the coalescing window slide the open
    /// transaction instead of sealing it, exactly as a sixty-frame drag does.
    pub fn apply(&self, document: &mut Document) -> bool {
        let moves: Vec<(&NodeId, Point)> = self
            .bodies
            .iter()
            .filter_map(|body| {
                let position = Point::new(body.x - body.width / 2.0, body.y - body.height / 2.0);
                let current = document.node(&body.id)?.position;
                let shifted = (current.x - position.x).abs() > MOVE_EPSILON
                    || (current.y - position.y).abs() > MOVE_EPSILON;
                shifted.then_some((&body.id, position))
            })
            .collect();
        if moves.is_empty() {
            return false;
        }
        document.transaction_of(EditKind::Move, |document| {
            for (id, position) in moves {
                document.set_position(id, position);
            }
        });
        true
    }

    fn link_force(&mut self) {
        for index in 0..self.links.len() {
            let link = self.links[index];
            let (source, target) = (link.source, link.target);
            let mut x = self.bodies[target].x + self.bodies[target].vx
                - self.bodies[source].x
                - self.bodies[source].vx;
            let mut y = self.bodies[target].y + self.bodies[target].vy
                - self.bodies[source].y
                - self.bodies[source].vy;
            if x == 0.0 {
                x = self.random.jiggle();
            }
            if y == 0.0 {
                y = self.random.jiggle();
            }
            // `x * x + y * y` rather than `hypot`: d3 computes it this way, and the plain
            // form is IEEE-exact where a libm `hypot` is only correctly rounded by convention.
            let length = (x * x + y * y).sqrt();
            let scale = (length - link.distance) / length * self.alpha * LINK_STRENGTH;
            x *= scale;
            y *= scale;

            self.bodies[target].vx -= x * link.bias;
            self.bodies[target].vy -= y * link.bias;
            let inverse = 1.0 - link.bias;
            self.bodies[source].vx += x * inverse;
            self.bodies[source].vy += y * inverse;
        }
    }

    /// `forceManyBody` without the Barnes-Hut approximation — the exact sum d3's quadtree
    /// estimates. A canvas holds tens of nodes and a schema page a few hundred at worst, where
    /// the quadratic loop is cheaper than building a quadtree per tick and has no theta to
    /// make the result depend on insertion order.
    fn charge_force(&mut self) {
        let positions: Vec<(f64, f64)> = self.bodies.iter().map(|body| (body.x, body.y)).collect();
        let weight = CHARGE_STRENGTH * self.alpha;
        for (index, (origin_x, origin_y)) in positions.iter().enumerate() {
            let mut vx = 0.0;
            let mut vy = 0.0;
            for (other, (other_x, other_y)) in positions.iter().enumerate() {
                if other == index {
                    continue;
                }
                let mut x = other_x - origin_x;
                let mut y = other_y - origin_y;
                let mut squared = x * x + y * y;
                if squared >= CHARGE_DISTANCE_MAX_SQUARED {
                    continue;
                }
                if x == 0.0 {
                    x = self.random.jiggle();
                    squared += x * x;
                }
                if y == 0.0 {
                    y = self.random.jiggle();
                    squared += y * y;
                }
                if squared < CHARGE_DISTANCE_MIN_SQUARED {
                    squared = (CHARGE_DISTANCE_MIN_SQUARED * squared).sqrt();
                }
                vx += x * weight / squared;
                vy += y * weight / squared;
            }
            self.bodies[index].vx += vx;
            self.bodies[index].vy += vy;
        }
    }

    fn anchor_force(&mut self) {
        let alpha = self.alpha;
        for (body, anchor) in self.bodies.iter_mut().zip(self.anchors.iter()) {
            body.vx += (anchor.x - body.x) * ANCHOR_STRENGTH * alpha;
            body.vy += (anchor.y - body.y) * ANCHOR_STRENGTH * alpha;
        }
    }

    /// Rectangular collision. d3's `forceCollide` treats nodes as circles, which leaves a wide
    /// node surrounded by the empty space its diagonal claims.
    fn collide_force(&mut self) {
        for _ in 0..COLLIDE_ITERATIONS {
            for first in 0..self.bodies.len() {
                for second in (first + 1)..self.bodies.len() {
                    self.resolve_overlap(first, second);
                }
            }
        }
    }

    fn resolve_overlap(&mut self, first: usize, second: usize) {
        let (ax, ay) = (self.bodies[first].x, self.bodies[first].y);
        let (bx, by) = (self.bodies[second].x, self.bodies[second].y);
        let dx = bx - ax;
        let dy = by - ay;
        let overlap_x = f64::midpoint(self.bodies[first].width, self.bodies[second].width)
            + COLLIDE_PADDING
            - dx.abs();
        let overlap_y = f64::midpoint(self.bodies[first].height, self.bodies[second].height)
            + COLLIDE_PADDING
            - dy.abs();
        if overlap_x <= 0.0 || overlap_y <= 0.0 {
            return;
        }
        // Push along the shallower axis: separating a pair that barely overlaps sideways by
        // shoving them apart vertically would undo the layout the springs just found.
        if overlap_x < overlap_y {
            let shift = overlap_x / 2.0 * COLLIDE_STRENGTH * if dx < 0.0 { -1.0 } else { 1.0 };
            self.bodies[first].x = ax - shift;
            self.bodies[second].x = bx + shift;
        } else {
            let shift = overlap_y / 2.0 * COLLIDE_STRENGTH * if dy < 0.0 { -1.0 } else { 1.0 };
            self.bodies[first].y = ay - shift;
            self.bodies[second].y = by + shift;
        }
    }
}

fn max_dimension(body: &Body) -> f64 {
    body.width.max(body.height)
}

#[cfg(test)]
mod tests {
    use super::Layout;
    use crate::model::Document;
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{CanvasDocument, Node, NodeId, NodeType};

    /// `count` query nodes a pixel apart, which is the worst case the simulation has to cope
    /// with: it is what the jiggle exists for and what the collision force has to pull apart.
    fn document_with(count: usize) -> (Document, Vec<NodeId>) {
        let mut document = Document::load(CanvasDocument::empty());
        for index in 0..count {
            #[allow(clippy::cast_precision_loss, reason = "a test with a handful of nodes")]
            let offset = index as f64;
            document.create_node(
                NodeType::Query,
                Rect::new(Point::new(offset, offset), Size::new(350.0, 240.0)),
            );
        }
        let ids = document
            .nodes()
            .iter()
            .map(|node| node.id.clone())
            .collect();
        (document, ids)
    }

    /// Every node hanging off the first, so the whole page is one island sharing one anchor.
    /// Disconnected nodes each get an island of their own and the packer alone spaces them out,
    /// which is why a connected page is the one that exercises the forces at all.
    fn star(count: usize) -> (Document, Vec<NodeId>) {
        let (mut document, ids) = document_with(count);
        for target in &ids[1..] {
            document.connect(&ids[0], target);
        }
        (document, ids)
    }

    /// The view's loop: step, write back, repeat until at rest.
    fn run(document: &mut Document) {
        let mut layout = Layout::of_active_page(document).expect("enough nodes to arrange");
        while !layout.is_finished() {
            layout.step();
            layout.apply(document);
        }
    }

    fn positions(document: &Document) -> Vec<Point> {
        document.nodes().iter().map(|node| node.position).collect()
    }

    #[test]
    fn fewer_than_two_nodes_is_nothing_to_arrange() {
        assert!(Layout::of_active_page(&document_with(0).0).is_none());
        assert!(Layout::of_active_page(&document_with(1).0).is_none());
        assert!(Layout::of_active_page(&document_with(2).0).is_some());
    }

    #[test]
    fn it_converges() {
        let mut layout = Layout::of_active_page(&star(6).0).expect("enough nodes to arrange");
        let mut ticks = 0;
        while !layout.is_finished() {
            layout.step();
            ticks += 1;
            assert!(ticks < 1000, "the simulation has to cool down");
        }
        assert!(ticks > 10, "and not in a single tick: {ticks}");
    }

    #[test]
    fn the_same_document_always_settles_in_the_same_place() {
        let (mut first, _) = star(5);
        let (mut second, _) = star(5);
        run(&mut first);
        run(&mut second);
        assert_eq!(positions(&first), positions(&second));
    }

    /// What the user actually asked for. A page whose nodes all hang together has nothing but
    /// the forces keeping them apart, so this is the test the collision force has to pass.
    #[test]
    fn a_connected_page_settles_without_overlapping_cards() {
        let (mut document, _) = star(12);
        run(&mut document);

        let bounds: Vec<Rect> = document.nodes().iter().map(Node::bounds).collect();
        for (index, first) in bounds.iter().enumerate() {
            for second in &bounds[index + 1..] {
                assert!(
                    !first.intersects(*second),
                    "{first:?} still overlaps {second:?}"
                );
            }
        }
    }

    /// A run is one undo step however many ticks it took, because every tick's writes are
    /// `EditKind::Move` and land inside the coalescing window.
    #[test]
    fn a_whole_run_undoes_in_one_press() {
        let (mut document, _) = star(4);
        let before = positions(&document);
        document.checkpoint();

        run(&mut document);
        document.checkpoint();
        assert!(document.undo(), "the run is undoable");
        assert_eq!(
            before,
            positions(&document),
            "one press puts every node back"
        );
    }

    #[test]
    fn a_run_re_fits_the_camera_every_eight_ticks() {
        let mut layout = Layout::of_active_page(&star(3).0).expect("enough nodes to arrange");
        let refits: Vec<usize> = (0..16).filter(|_| layout.step()).collect();
        assert_eq!(refits, vec![7, 15], "the 8th and the 16th tick");
    }

    /// Where d3 puts this graph with the reference's constants. Every force, every constant and
    /// the integration order feed these numbers, so any drift from `forceSimulation.ts` shows up
    /// here — which is the only way to catch a spring strength that is quietly wrong but still
    /// converges to something plausible.
    ///
    /// Exact across machines: the simulation uses only `+ - * /`, `sqrt` and `mul_add`, all of
    /// which IEEE-754 pins to a single result. The tolerance is for the recorded decimals.
    #[test]
    fn a_small_graph_settles_where_the_reference_constants_put_it() {
        let (mut document, ids) = document_with(5);
        document.connect(&ids[0], &ids[1]);
        document.connect(&ids[1], &ids[2]);
        document.connect(&ids[0], &ids[3]);
        run(&mut document);

        let expected = [
            (-376.919, -599.164),
            (-392.689, -267.331),
            (20.915, -560.610),
            (5.141, -272.740),
            (-513.228, 539.708),
        ];
        for (settled, (x, y)) in positions(&document).iter().zip(expected) {
            assert!(
                (settled.x - x).abs() < 0.01 && (settled.y - y).abs() < 0.01,
                "{settled:?} is not ({x}, {y})"
            );
        }
    }
}
