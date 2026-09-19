//! The zoom cross-fade that hands the canvas from nodes to region beacons — the port of
//! `wayfinding/crossFade.ts`.
//!
//! `t` is 0 while the camera is close enough to read a node and 1 once it is far enough that
//! only the region labels are worth drawing. Everything wayfinding paints reads it: beacons
//! fade in with `t`, edge peekers fade out with it, the confirmed halos take it as their
//! opacity, and past [`DIM_THRESHOLD_T`] the nodes themselves recede.
//!
//! [`crate::lod`] draws its own line a little earlier (0.32) and for a different reason — it
//! stops *building* bodies rather than dimming them — but the two describe the same moment.

/// At and above this zoom the camera is reading, and wayfinding paints nothing.
pub const BEACON_FADE_START: f64 = 0.35;
/// How much further out the camera has to go for the hand-off to finish.
pub const BEACON_FADE_RANGE: f64 = 0.14;
/// Past this `t`, nodes and edges recede behind the beacons.
pub const DIM_THRESHOLD_T: f64 = 0.4;
/// Past this `t`, a beacon stops being a label and starts taking the pointer.
pub const INTERACTIVE_THRESHOLD_T: f64 = 0.35;
/// Peekers fade out a touch faster than beacons fade in, so the two never fight mid-fade.
pub const PEEK_FADE_FACTOR: f64 = 1.4;
/// Below this opacity a peeker is decoration, and stops taking the pointer.
pub const INTERACTIVE_OPACITY: f64 = 0.2;

/// What nodes and edges fade to once [`DIM_THRESHOLD_T`] is crossed (`wayfinding.css`).
pub const NODE_DIM: f32 = 0.42;
pub const EDGE_DIM: f32 = 0.35;

/// How far through the hand-off `zoom` is: 0 while nodes are readable, 1 in overview.
#[must_use]
pub fn cross_fade(zoom: f64) -> f64 {
    ((BEACON_FADE_START - zoom) / BEACON_FADE_RANGE).clamp(0.0, 1.0)
}

/// How much of a peeker to draw at `t`, before the moving/hover gate is applied.
#[must_use]
pub fn peek_opacity(t: f64) -> f64 {
    (1.0 - t * PEEK_FADE_FACTOR).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{BEACON_FADE_START, cross_fade, peek_opacity};

    #[test]
    fn a_readable_camera_is_not_fading_at_all() {
        assert!((cross_fade(1.0) - 0.0).abs() < f64::EPSILON);
        assert!((cross_fade(BEACON_FADE_START) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_fade_completes_one_range_below_the_start() {
        assert!((cross_fade(0.21) - 1.0).abs() < 1e-9);
        assert!((cross_fade(0.1) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_midpoint_is_half_way_through() {
        assert!((cross_fade(0.28) - 0.5).abs() < 1e-9);
    }

    /// The peekers are gone before the beacons are fully in, which is the whole point of the
    /// factor: two layers saying the same thing at once reads as a double exposure.
    #[test]
    fn peekers_are_gone_before_the_beacons_arrive() {
        assert!(peek_opacity(cross_fade(0.35)) > 0.99);
        assert!((peek_opacity(0.72) - 0.0).abs() < f64::EPSILON);
    }
}
