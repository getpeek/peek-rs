//! The edge layer: one stroked cubic bezier per edge, painted between the dot grid and the
//! nodes so every curve runs behind the cards it connects.
//!
//! The geometry is `peek_canvas::edge`, shared with hit-testing so what you press is what you
//! see. Only the conversion to gpui pixels lives here.

use gpui_kit::{Bounds, Hsla, Path, PathBuilder, Pixels, Point, Window};
use peek_canvas::Camera;
use peek_canvas::edge::EdgeCurve;
use peek_theme::EdgeState;

use super::convert::pixels;
use super::screen_point;

/// `node.css` stroke widths, in world units at zoom 1.
const STROKE_WIDTH: f64 = 2.6;
/// `.connection-active`: an edge touching a selected node is thicker as well as brighter.
const ACTIVE_STROKE_WIDTH: f64 = 3.4;

pub(crate) struct EdgeItem {
    pub curve: EdgeCurve,
    pub color: Hsla,
    pub width: f64,
}

#[must_use]
pub(crate) fn stroke_width(state: EdgeState) -> f64 {
    match state {
        EdgeState::Resting | EdgeState::Selected => STROKE_WIDTH,
        EdgeState::ConnectionActive => ACTIVE_STROKE_WIDTH,
    }
}

pub(crate) fn paint(
    bounds: Bounds<Pixels>,
    camera: Camera,
    items: &[EdgeItem],
    window: &mut Window,
) {
    for item in items {
        if let Some(path) = trace(item, camera, bounds.origin) {
            window.paint_path(path, item.color);
        }
    }
}

/// Path coordinates are raw pixels: the canvas' rem scope never reaches them, so the stroke
/// takes the zoom itself or edges would stay hairlines as the camera pulls in.
fn trace(item: &EdgeItem, camera: Camera, origin: Point<Pixels>) -> Option<Path<Pixels>> {
    let width = pixels(item.width * camera.zoom);
    if width <= Pixels::ZERO {
        return None;
    }
    let at = |world| screen_point(camera, world) + origin;
    let mut builder = PathBuilder::stroke(width);
    builder.move_to(at(item.curve.start));
    builder.cubic_bezier_to(
        at(item.curve.end),
        at(item.curve.control_start),
        at(item.curve.control_end),
    );
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Pixels, point, px};
    use peek_canvas::edge::curve_between;
    use peek_canvas::{Camera, Point, Rect, Size};
    use peek_theme::EdgeState;

    use super::{EdgeItem, stroke_width, trace};

    fn item(width: f64) -> EdgeItem {
        let source = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let target = Rect::new(Point::new(400.0, 0.0), Size::new(100.0, 100.0));
        EdgeItem {
            curve: curve_between(source, target),
            color: gpui_kit::white(),
            width,
        }
    }

    #[test]
    fn a_stroked_edge_tessellates_where_the_camera_puts_it() {
        let origin = point(px(60.0), px(30.0));

        let path = trace(
            &item(stroke_width(EdgeState::Resting)),
            Camera::default(),
            origin,
        )
        .expect("lyon strokes a two-point bezier");

        assert!(!path.vertices.is_empty(), "the stroke has geometry");
        let left = path
            .vertices
            .iter()
            .fold(Pixels::MAX, |left, vertex| left.min(vertex.xy_position.x));
        assert!(
            left >= px(150.0) && left < px(170.0),
            "it starts at the source's right border, offset by the pane: {left:?}"
        );
    }

    #[test]
    fn the_stroke_thickens_with_the_camera() {
        let thin = trace(&item(2.6), Camera::default(), point(px(0.0), px(0.0)))
            .expect("a path at zoom 1");
        let zoomed = Camera {
            zoom: 4.0,
            ..Camera::default()
        };
        let thick = trace(&item(2.6), zoomed, point(px(0.0), px(0.0))).expect("a path at zoom 4");

        let span = |path: &gpui_kit::Path<Pixels>| {
            let (top, bottom) = path
                .vertices
                .iter()
                .fold((Pixels::MAX, Pixels::MIN), |acc, v| {
                    (acc.0.min(v.xy_position.y), acc.1.max(v.xy_position.y))
                });
            bottom - top
        };

        assert!(
            span(&thick) > span(&thin) * 3.0,
            "rem scoping never reaches path coordinates, so the stroke scales itself"
        );
    }

    #[test]
    fn a_collapsed_camera_paints_nothing() {
        let flat = Camera {
            zoom: 0.0,
            ..Camera::default()
        };

        assert!(trace(&item(2.6), flat, point(px(0.0), px(0.0))).is_none());
    }
}
