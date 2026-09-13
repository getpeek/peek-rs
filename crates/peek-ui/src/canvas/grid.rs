//! The dot grid (`CanvasBackground.tsx`: 28 world units apart), thinned by
//! doubling the gap as the camera zooms out so it never turns into noise.

use gpui_kit::{Bounds, Hsla, Pixels, Window, fill, point, px, size};
use peek_canvas::Camera;
use peek_canvas::grid::grid_step;

use super::screen_point;

pub(crate) fn paint_dot_grid(
    bounds: Bounds<Pixels>,
    camera: Camera,
    color: Hsla,
    window: &mut Window,
) {
    let step = grid_step(camera.zoom);
    let pane = peek_canvas::Size::new(
        f64::from(f32::from(bounds.size.width)),
        f64::from(f32::from(bounds.size.height)),
    );
    let visible = camera.visible_world_rect(pane);
    let first_x = (visible.min().x / step.world_gap).floor() * step.world_gap;
    let first_y = (visible.min().y / step.world_gap).floor() * step.world_gap;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "dot sizes are a few pixels"
    )]
    let dot = px((camera.zoom as f32 * 1.25).clamp(1.25, 2.0));
    let half = dot / 2.0;

    let mut y = first_y;
    while y <= visible.max().y {
        let mut x = first_x;
        while x <= visible.max().x {
            let center = screen_point(camera, peek_canvas::Point::new(x, y)) + bounds.origin;
            let origin = point(center.x - half, center.y - half);
            window.paint_quad(fill(Bounds::new(origin, size(dot, dot)), color).corner_radii(half));
            x += step.world_gap;
        }
        y += step.world_gap;
    }
}
