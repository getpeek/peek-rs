//! Animated camera moves. The world centre is interpolated linearly and the zoom
//! geometrically (in log space) so the focal point never drifts sideways mid-flight, which
//! is what a naive lerp of `(x, y, zoom)` does. Every flight in the app is 150–600 ms.

use std::time::Duration;

use peek_document::geometry::{Point, Size};

use crate::camera::Camera;

/// Durations from `hooks/useCanvas.ts`, `ZoomIndicator.tsx` and `useRegionActions.ts`.
pub mod durations {
    use std::time::Duration;

    pub const ZOOM_TO_NODE: Duration = Duration::from_millis(300);
    pub const FIT_NODES: Duration = Duration::from_millis(300);
    pub const PAN_TO: Duration = Duration::from_millis(300);
    pub const FIT_VIEW: Duration = Duration::from_millis(300);
    pub const FIT_SELECTED: Duration = Duration::from_millis(200);
    pub const RESET_ZOOM: Duration = Duration::from_millis(200);
    pub const SET_ZOOM: Duration = Duration::from_millis(200);
    pub const ZOOM_BUTTON: Duration = Duration::from_millis(150);
    pub const ZOOM_TO_FIT_BUTTON: Duration = Duration::from_millis(250);
    pub const REGION_FLY: Duration = Duration::from_millis(600);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    #[default]
    EaseInOutCubic,
    EaseOutCubic,
    Linear,
}

impl Easing {
    #[must_use]
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
            Self::EaseInOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraFlight {
    pub from: Camera,
    pub to: Camera,
    pub duration: Duration,
    pub easing: Easing,
    viewport: Size,
}

impl CameraFlight {
    #[must_use]
    pub fn new(from: Camera, to: Camera, duration: Duration, viewport: Size) -> Self {
        Self {
            from,
            to,
            duration,
            easing: Easing::default(),
            viewport,
        }
    }

    #[must_use]
    pub fn with_easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Normalised progress for `elapsed` wall time, `1.0` once the flight is over.
    #[must_use]
    pub fn progress(&self, elapsed: Duration) -> f64 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn is_finished(&self, elapsed: Duration) -> bool {
        elapsed >= self.duration
    }

    /// The camera at normalised time `t` (easing applied here).
    #[must_use]
    pub fn sample(&self, t: f64) -> Camera {
        let eased = self.easing.apply(t);
        if eased >= 1.0 {
            return self.to;
        }
        let center_from = self.center_of(self.from);
        let center_to = self.center_of(self.to);
        let center = center_from + (center_to - center_from).scaled(eased);
        let zoom = self.from.zoom * (self.to.zoom / self.from.zoom).powf(eased);
        Camera::centered_on(center, zoom, self.viewport)
    }

    fn center_of(&self, camera: Camera) -> Point {
        camera.screen_to_world(Point::new(
            self.viewport.width / 2.0,
            self.viewport.height / 2.0,
        ))
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "tests assert exact clamped constants")]
mod tests {
    use super::*;

    fn flight() -> CameraFlight {
        let viewport = Size::new(1000.0, 600.0);
        let from = Camera::centered_on(Point::new(0.0, 0.0), 0.5, viewport);
        let to = Camera::centered_on(Point::new(800.0, -200.0), 2.0, viewport);
        CameraFlight::new(from, to, Duration::from_millis(300), viewport)
    }

    #[test]
    fn endpoints_are_exact() {
        let flight = flight();
        assert!(flight.sample(0.0).approx_eq(flight.from));
        assert!(flight.sample(1.0).approx_eq(flight.to));
        assert_eq!(flight.progress(Duration::from_millis(150)), 0.5);
        assert!(flight.is_finished(Duration::from_millis(300)));
    }

    #[test]
    fn zoom_is_monotonic_and_centre_moves_straight() {
        let flight = flight();
        let mut last_zoom = 0.0;
        for step in 0..=10 {
            let t = f64::from(step) / 10.0;
            let camera = flight.sample(t);
            assert!(camera.zoom >= last_zoom, "zoom went backwards at {t}");
            last_zoom = camera.zoom;
            let center = camera.screen_to_world(Point::new(500.0, 300.0));
            // Centre stays on the segment (0,0)→(800,-200): y = -x / 4.
            assert!((center.y + center.x / 4.0).abs() < 0.5, "{center:?}");
        }
    }

    #[test]
    fn easing_is_bounded_and_symmetric() {
        assert_eq!(Easing::EaseInOutCubic.apply(0.0), 0.0);
        assert_eq!(Easing::EaseInOutCubic.apply(1.0), 1.0);
        assert!((Easing::EaseInOutCubic.apply(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(Easing::Linear.apply(2.0), 1.0);
    }
}
