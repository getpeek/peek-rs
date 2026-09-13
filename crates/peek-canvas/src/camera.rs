//! React Flow viewport semantics: `screen = world * zoom + pan`.

use peek_document::Viewport;
use peek_document::geometry::{Point, Rect, Size};

pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub pan: Point,
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: Point::default(),
            zoom: 1.0,
        }
    }
}

impl Camera {
    #[must_use]
    pub fn from_viewport(viewport: Viewport) -> Self {
        Self {
            pan: Point::new(viewport.x, viewport.y),
            zoom: Self::clamp_zoom(viewport.zoom),
        }
    }

    #[must_use]
    pub fn to_viewport(self) -> Viewport {
        Viewport {
            x: self.pan.x,
            y: self.pan.y,
            zoom: self.zoom,
        }
    }

    #[must_use]
    pub fn clamp_zoom(zoom: f64) -> f64 {
        if zoom.is_finite() {
            zoom.clamp(MIN_ZOOM, MAX_ZOOM)
        } else {
            1.0
        }
    }

    #[must_use]
    pub fn world_to_screen(self, world: Point) -> Point {
        world.scaled(self.zoom) + self.pan
    }

    #[must_use]
    pub fn screen_to_world(self, screen: Point) -> Point {
        (screen - self.pan).scaled(1.0 / self.zoom)
    }

    #[must_use]
    pub fn world_rect_to_screen(self, rect: Rect) -> Rect {
        Rect::new(
            self.world_to_screen(rect.origin),
            rect.size.scaled(self.zoom),
        )
    }

    /// The world-space rectangle a viewport of `size` pixels shows.
    #[must_use]
    pub fn visible_world_rect(self, size: Size) -> Rect {
        Rect::from_corners(
            self.screen_to_world(Point::default()),
            self.screen_to_world(Point::new(size.width, size.height)),
        )
    }

    #[must_use]
    pub fn panned_by(self, delta_screen: Point) -> Self {
        Self {
            pan: self.pan + delta_screen,
            zoom: self.zoom,
        }
    }

    /// Changes zoom so the world point under `anchor_screen` stays put.
    #[must_use]
    pub fn zoomed_about(self, anchor_screen: Point, new_zoom: f64) -> Self {
        let zoom = Self::clamp_zoom(new_zoom);
        let world = self.screen_to_world(anchor_screen);
        Self {
            pan: anchor_screen - world.scaled(zoom),
            zoom,
        }
    }

    #[must_use]
    pub fn zoomed_by_about(self, anchor_screen: Point, factor: f64) -> Self {
        self.zoomed_about(anchor_screen, self.zoom * factor)
    }

    /// Places `world` at the centre of a viewport of `size` pixels (React Flow `setCenter`).
    #[must_use]
    pub fn centered_on(world: Point, zoom: f64, size: Size) -> Self {
        let zoom = Self::clamp_zoom(zoom);
        let center = Point::new(size.width / 2.0, size.height / 2.0);
        Self {
            pan: center - world.scaled(zoom),
            zoom,
        }
    }

    /// React Flow `getViewportForBounds`: the zoom that fits `bounds` with `padding` (a
    /// fraction of the bounds, 0.1 = 10 %) clamped to `[min_zoom, max_zoom]`, centred.
    #[must_use]
    pub fn fit_bounds(bounds: Rect, size: Size, fit: FitOptions) -> Self {
        let padded_width = bounds.size.width * (1.0 + fit.padding);
        let padded_height = bounds.size.height * (1.0 + fit.padding);
        let zoom_x = if padded_width > 0.0 {
            size.width / padded_width
        } else {
            fit.max_zoom
        };
        let zoom_y = if padded_height > 0.0 {
            size.height / padded_height
        } else {
            fit.max_zoom
        };
        let zoom = zoom_x.min(zoom_y).clamp(fit.min_zoom, fit.max_zoom);
        Self::centered_on(bounds.center(), zoom, size)
    }

    /// Where a world point lands as a centre-relative screen point; used to check flights.
    #[must_use]
    pub fn approx_eq(self, other: Self) -> bool {
        (self.zoom - other.zoom).abs() < 1e-4 && self.pan.distance_to(other.pan) < 0.05
    }
}

/// Parameters for [`Camera::fit_bounds`]; React Flow's defaults are `padding 0.1`,
/// `min 0.5`, `max 2`, but Peek always passes `max_zoom: 1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitOptions {
    pub padding: f64,
    pub min_zoom: f64,
    pub max_zoom: f64,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            padding: 0.1,
            min_zoom: MIN_ZOOM,
            max_zoom: 1.0,
        }
    }
}

impl FitOptions {
    #[must_use]
    pub fn padding(padding: f64) -> Self {
        Self {
            padding,
            ..Self::default()
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "tests assert exact clamped constants")]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        a.distance_to(b) < 1e-3
    }

    #[test]
    fn zooming_about_an_anchor_keeps_its_world_point_fixed() {
        let camera = Camera {
            pan: Point::new(120.0, -40.0),
            zoom: 0.8,
        };
        for (anchor, factor) in [
            (Point::new(0.0, 0.0), 1.5),
            (Point::new(640.0, 400.0), 0.3),
            (Point::new(13.0, 999.0), 4.0),
        ] {
            let world_before = camera.screen_to_world(anchor);
            let zoomed = camera.zoomed_by_about(anchor, factor);
            let world_after = zoomed.screen_to_world(anchor);
            assert!(close(world_before, world_after), "{anchor:?} x{factor}");
        }
    }

    #[test]
    fn zoom_is_clamped_to_the_react_flow_range() {
        let camera = Camera::default();
        assert_eq!(
            camera.zoomed_by_about(Point::default(), 100.0).zoom,
            MAX_ZOOM
        );
        assert_eq!(
            camera.zoomed_by_about(Point::default(), 0.0001).zoom,
            MIN_ZOOM
        );
        assert_eq!(Camera::clamp_zoom(f64::NAN), 1.0);
    }

    #[test]
    fn screen_and_world_round_trip() {
        let camera = Camera {
            pan: Point::new(-300.0, 55.0),
            zoom: 2.5,
        };
        let world = Point::new(17.0, -8.0);
        assert!(close(
            camera.screen_to_world(camera.world_to_screen(world)),
            world
        ));
    }

    #[test]
    fn centered_on_puts_the_point_in_the_middle() {
        let size = Size::new(1200.0, 800.0);
        let camera = Camera::centered_on(Point::new(50.0, 50.0), 1.0, size);
        assert!(close(
            camera.world_to_screen(Point::new(50.0, 50.0)),
            Point::new(600.0, 400.0)
        ));
    }

    #[test]
    fn fit_bounds_respects_max_zoom_and_centres() {
        let size = Size::new(1200.0, 800.0);
        let small = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let camera = Camera::fit_bounds(small, size, FitOptions::default());
        assert_eq!(camera.zoom, 1.0, "small content must not zoom past 1");
        assert!(close(
            camera.world_to_screen(small.center()),
            Point::new(600.0, 400.0)
        ));

        let huge = Rect::new(Point::new(-5000.0, 0.0), Size::new(10000.0, 4000.0));
        let camera = Camera::fit_bounds(huge, size, FitOptions::default());
        assert!(camera.zoom < 0.12 && camera.zoom >= MIN_ZOOM);
        let visible = camera.visible_world_rect(size);
        assert!(visible.contains(huge.min()) && visible.contains(huge.max()));
    }

    #[test]
    fn visible_rect_grows_as_zoom_shrinks() {
        let size = Size::new(1000.0, 500.0);
        let camera = Camera {
            pan: Point::default(),
            zoom: 0.1,
        };
        let visible = camera.visible_world_rect(size);
        assert!((visible.size.width - 10000.0).abs() < 1e-2);
        assert!((visible.size.height - 5000.0).abs() < 1e-2);
    }
}
