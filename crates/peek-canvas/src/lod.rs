//! How much of a node is worth drawing at the camera's current zoom.
//!
//! Culling bounds the work only while nodes leave the viewport; zooming out does the opposite,
//! because more cards fit the screen the further back the camera goes. Past the point where a
//! card's body is too small to read, building it is pure cost: a result node three hundred
//! pixels wide still lays out a toolbar and a few hundred cells nobody can see.
//!
//! The reference draws the same line. Below `BEACON_FADE_START` (0.35) in
//! `~/labs/peek/src/canvas/wayfinding/crossFade.ts` it dims the nodes and hands the board over
//! to region beacons, on the reasoning that you have stopped reading nodes and started
//! navigating between them.
//!
//! This supersedes the decision recorded in `docs/decisions.md` that gpui needs no level of
//! detail. That was an assumption about the renderer; the cost is in building and laying out
//! the element trees, which no renderer avoids.

/// Below this the camera is navigating, not reading, and bodies stop being built.
pub const REDUCE_BELOW: f64 = 0.32;
/// And above this they come back. The gap is deliberate: a single threshold would rebuild every
/// visible body twice a frame while a slow zoom sat on it.
pub const RESTORE_ABOVE: f64 = 0.38;

/// How much of a node to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detail {
    /// The kind's real body.
    #[default]
    Full,
    /// The shell alone — card, kind indicator and title — with no body.
    Reduced,
}

impl Detail {
    #[must_use]
    pub fn is_reduced(self) -> bool {
        self == Self::Reduced
    }
}

/// The tier for `zoom`, given the tier the last frame settled on.
///
/// Takes the previous tier because the thresholds overlap: inside the band the answer is
/// "whatever it already was", which is what keeps a hovering zoom from flapping.
#[must_use]
pub fn detail(zoom: f64, previous: Detail) -> Detail {
    if zoom <= REDUCE_BELOW {
        return Detail::Reduced;
    }
    if zoom >= RESTORE_ABOVE {
        return Detail::Full;
    }
    previous
}

#[cfg(test)]
mod tests {
    use super::{Detail, REDUCE_BELOW, RESTORE_ABOVE, detail};

    #[test]
    fn a_readable_camera_builds_real_bodies() {
        assert_eq!(detail(1.0, Detail::Reduced), Detail::Full);
        assert_eq!(detail(4.0, Detail::Full), Detail::Full);
    }

    #[test]
    fn a_distant_camera_stops_building_them() {
        assert_eq!(detail(0.1, Detail::Full), Detail::Reduced);
        assert_eq!(detail(REDUCE_BELOW, Detail::Full), Detail::Reduced);
    }

    #[test]
    fn the_band_holds_whichever_tier_it_was_entered_from() {
        let middle = f64::midpoint(REDUCE_BELOW, RESTORE_ABOVE);
        assert_eq!(detail(middle, Detail::Full), Detail::Full);
        assert_eq!(detail(middle, Detail::Reduced), Detail::Reduced);
    }

    /// The reason the band exists: a zoom that creeps across it must switch once, not per frame.
    #[test]
    fn creeping_across_the_band_switches_exactly_once() {
        let mut tier = Detail::Full;
        let mut switches = 0;
        for step in 0..=200 {
            let zoom = 0.45 - 0.20 * f64::from(step) / 200.0;
            let next = detail(zoom, tier);
            if next != tier {
                switches += 1;
            }
            tier = next;
        }
        assert_eq!(tier, Detail::Reduced);
        assert_eq!(switches, 1, "the band must not be crossed more than once");
    }

    /// A jitter of a few per cent — a trackpad resting mid-pinch — must not toggle the tier.
    #[test]
    fn jitter_inside_the_band_never_toggles() {
        let middle = f64::midpoint(REDUCE_BELOW, RESTORE_ABOVE);
        let mut tier = Detail::Full;
        for step in 0..100 {
            let jitter = if step % 2 == 0 { 1.01 } else { 0.99 };
            tier = detail(middle * jitter, tier);
            assert_eq!(tier, Detail::Full);
        }
    }
}
