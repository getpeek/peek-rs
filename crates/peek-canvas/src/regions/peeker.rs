//! Where an off-screen region's label sits — the port of `regionGeometry.ts:placePeeker`.
//!
//! A peeker is a compass needle, not a map: it is pinned to the viewport edge nearest the
//! region and rotated toward where the region actually is.

use peek_document::geometry::{Point, Rect, Size};

use crate::camera::Camera;

/// `.wf-peeker`'s fixed width, and the height the CSS lays one out at.
pub const PEEKER_WIDTH: f64 = 212.0;
pub const PEEKER_HEIGHT: f64 = 50.0;

const INSET_X: f64 = 16.0;
/// Clear of the page tabs up top and the toolbar down below.
const INSET_TOP: f64 = 64.0;
const INSET_BOTTOM: f64 = 62.0;
/// A region only counts as off-screen once its box clears this much slack, so a peeker does
/// not flicker in while the region is half visible at an edge.
const OFFSCREEN_SLACK: f64 = 40.0;

/// Which end of the label the arrow sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// A placed peeker, in pane coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub origin: Point,
    /// Degrees clockwise from east, pointing at the region's true centre.
    pub angle: f64,
    pub align: Align,
}

/// Where to pin a label for `bbox`, or `None` while the region is still (partly) on screen.
#[must_use]
pub fn place(bbox: Rect, camera: Camera, pane: Size) -> Option<Placement> {
    let screen = camera.world_rect_to_screen(bbox);
    let (min, max) = (screen.min(), screen.max());
    let on_screen = max.x > OFFSCREEN_SLACK
        && min.x < pane.width - OFFSCREEN_SLACK
        && max.y > OFFSCREEN_SLACK
        && min.y < pane.height - OFFSCREEN_SLACK;
    if on_screen {
        return None;
    }

    let center = screen.center();
    let origin = Point::new(
        clamp(
            center.x - PEEKER_WIDTH / 2.0,
            INSET_X,
            pane.width - PEEKER_WIDTH - INSET_X,
        ),
        clamp(
            center.y - PEEKER_HEIGHT / 2.0,
            INSET_TOP,
            pane.height - PEEKER_HEIGHT - INSET_BOTTOM,
        ),
    );
    let label_center = Point::new(
        origin.x + PEEKER_WIDTH / 2.0,
        origin.y + PEEKER_HEIGHT / 2.0,
    );
    let angle = (center.y - label_center.y)
        .atan2(center.x - label_center.x)
        .to_degrees();
    let align = if center.x > pane.width - PEEKER_WIDTH {
        Align::Right
    } else {
        Align::Left
    };

    Some(Placement {
        origin,
        angle,
        align,
    })
}

/// `lo` wins a pane too small to hold the label, which is what `Math.max(lo, Math.min(hi, v))`
/// does — `f64::clamp` panics when the bounds cross.
fn clamp(value: f64, lo: f64, hi: f64) -> f64 {
    value.min(hi).max(lo)
}

#[cfg(test)]
mod tests {
    use super::{Align, PEEKER_WIDTH, place};
    use crate::camera::Camera;
    use peek_document::geometry::{Point, Rect, Size};

    fn pane() -> Size {
        Size::new(1200.0, 800.0)
    }

    fn box_at(x: f64, y: f64) -> Rect {
        Rect::new(Point::new(x, y), Size::new(300.0, 200.0))
    }

    #[test]
    fn a_visible_region_gets_no_peeker() {
        assert!(place(box_at(100.0, 100.0), Camera::default(), pane()).is_none());
    }

    /// The slack is what stops a peeker blinking on and off as a region grazes the edge.
    #[test]
    fn a_region_grazing_the_edge_still_counts_as_visible() {
        assert!(place(box_at(-250.0, 100.0), Camera::default(), pane()).is_none());
    }

    #[test]
    fn a_region_off_the_left_pins_to_the_left_inset() {
        let placed = place(box_at(-900.0, 300.0), Camera::default(), pane())
            .expect("well clear of the viewport");
        assert!((placed.origin.x - 16.0).abs() < f64::EPSILON);
        assert_eq!(placed.align, Align::Left);
        assert!(
            placed.angle.abs() > 90.0,
            "the arrow points back out to the left, got {}",
            placed.angle
        );
    }

    #[test]
    fn a_region_off_the_right_pins_right_and_flips_its_arrow() {
        let placed =
            place(box_at(2000.0, 300.0), Camera::default(), pane()).expect("well off to the right");
        assert!((placed.origin.x - (1200.0 - PEEKER_WIDTH - 16.0)).abs() < f64::EPSILON);
        assert_eq!(placed.align, Align::Right);
        assert!(placed.angle.abs() < 90.0);
    }

    #[test]
    fn a_region_above_the_viewport_clears_the_page_tabs() {
        let placed =
            place(box_at(400.0, -1500.0), Camera::default(), pane()).expect("well above the pane");
        assert!((placed.origin.y - 64.0).abs() < f64::EPSILON);
    }

    /// The camera decides what is off screen, not the world box: panning the region into view
    /// has to take its peeker down.
    #[test]
    fn panning_a_region_into_view_retires_its_peeker() {
        let far = box_at(2000.0, 300.0);
        assert!(place(far, Camera::default(), pane()).is_some());

        let followed = Camera::default().panned_by(Point::new(-1800.0, 0.0));
        assert!(place(far, followed, pane()).is_none());
    }
}
