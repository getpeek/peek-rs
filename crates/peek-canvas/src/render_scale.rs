//! The scale node *contents* are drawn at, as distinct from the camera's zoom.
//!
//! Nodes are real element trees laid out inside a rem scope of `base_rem * scale`, and gpui
//! caches both line layouts and rasterised glyphs against the exact font size they were
//! produced at. A zoom that moves continuously therefore produces a new font size every frame,
//! misses both caches for every label and every cell on screen, and re-shapes the whole
//! viewport each frame — the cost that made zooming out over result nodes drop frames.
//!
//! Snapping the *content* scale to a ladder converts that into a cache hit on all but the
//! frames that cross a rung. Node **boxes** keep the exact zoom, so positions, edges, hit
//! testing and selection rings are unaffected; only the type inside a card is at most one rung
//! off the box around it, which is not a difference the eye resolves.

use crate::camera::{MAX_ZOOM, MIN_ZOOM};

/// Rungs per doubling. Twelve puts neighbouring rungs about 6 % apart, so content is at most
/// ~3 % off the true zoom, and the whole `MIN_ZOOM..=MAX_ZOOM` range is about 64 distinct font
/// sizes rather than one per frame of every gesture.
const STEPS_PER_OCTAVE: f64 = 12.0;

/// The camera's `zoom` snapped to the content ladder.
///
/// The ladder is anchored on 1.0, so the default camera renders at exactly the base rem size.
#[must_use]
pub fn render_scale(zoom: f64) -> f64 {
    let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    if !zoom.is_finite() || zoom <= 0.0 {
        return 1.0;
    }
    let rung = (zoom.log2() * STEPS_PER_OCTAVE).round() / STEPS_PER_OCTAVE;
    rung.exp2().clamp(MIN_ZOOM, MAX_ZOOM)
}

#[cfg(test)]
mod tests {
    use super::{MAX_ZOOM, MIN_ZOOM, render_scale};

    /// Sampled the way a gesture moves the camera: finely, across the whole range.
    fn ladder() -> Vec<f64> {
        (0..=4000)
            .map(|step| MIN_ZOOM + (MAX_ZOOM - MIN_ZOOM) * f64::from(step) / 4000.0)
            .map(render_scale)
            .collect()
    }

    #[test]
    fn the_default_camera_renders_at_the_base_rem_size() {
        assert!(
            (render_scale(1.0) - 1.0).abs() < 1e-12,
            "the ladder must be anchored on 1.0, not near it"
        );
    }

    #[test]
    fn never_drifts_far_enough_from_the_zoom_to_see() {
        let mut worst: f64 = 0.0;
        for step in 0..=4000 {
            let zoom = MIN_ZOOM + (MAX_ZOOM - MIN_ZOOM) * f64::from(step) / 4000.0;
            worst = worst.max((render_scale(zoom) / zoom - 1.0).abs());
        }
        assert!(
            worst < 0.031,
            "content would visibly mismatch its box: {worst}"
        );
    }

    #[test]
    fn a_gesture_crosses_far_fewer_sizes_than_it_has_frames() {
        let mut rungs = ladder();
        rungs.dedup();
        assert!(
            rungs.len() < 80,
            "the whole zoom range must be a handful of font sizes, not thousands: {}",
            rungs.len()
        );
    }

    #[test]
    fn zooming_in_never_shrinks_the_content() {
        for pair in ladder().windows(2) {
            assert!(pair[1] >= pair[0], "the ladder must be monotonic: {pair:?}");
        }
    }

    #[test]
    fn stays_inside_the_camera_range() {
        for zoom in [f64::NAN, 0.0, -1.0, MIN_ZOOM, MAX_ZOOM, 1e9] {
            let scale = render_scale(zoom);
            assert!(
                (MIN_ZOOM..=MAX_ZOOM).contains(&scale),
                "{zoom} escaped the camera range as {scale}"
            );
        }
    }
}
