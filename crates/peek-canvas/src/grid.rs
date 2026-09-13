//! The dot grid's spacing, thinned as the camera zooms out so it never turns into noise.

/// Dot grid spacing in world units (`CanvasBackground.tsx`).
pub const GRID_GAP: f64 = 28.0;

/// Grid dots closer than this on screen are thinned out by doubling the gap.
pub const GRID_MIN_SCREEN_GAP: f64 = 12.0;

/// The on-screen grid spacing at `zoom`, doubling the world gap until dots are at least
/// [`GRID_MIN_SCREEN_GAP`] apart, plus the world gap that produced it.
#[must_use]
pub fn grid_step(zoom: f64) -> GridStep {
    let mut world_gap = GRID_GAP;
    while world_gap * zoom < GRID_MIN_SCREEN_GAP {
        world_gap *= 2.0;
    }
    GridStep {
        world_gap,
        screen_gap: world_gap * zoom,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridStep {
    pub world_gap: f64,
    pub screen_gap: f64,
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "tests assert exact clamped constants")]
mod tests {
    use super::*;

    #[test]
    fn grid_never_gets_denser_than_the_minimum() {
        for zoom in [4.0, 1.0, 0.5, 0.3, 0.1] {
            let step = grid_step(zoom);
            assert!(step.screen_gap >= GRID_MIN_SCREEN_GAP, "{zoom}");
            assert!(step.screen_gap < GRID_MIN_SCREEN_GAP * 2.0 || step.world_gap == GRID_GAP);
        }
    }
}
