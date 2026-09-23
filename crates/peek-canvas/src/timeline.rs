//! `useTimelineTrack.ts`: where the version-history timeline puts its dots, and how far it is
//! panned. In the same pixels the panel is pinned in — this chrome does not zoom.

/// Distance between neighbouring checkpoints.
pub const TRACK_PITCH: f32 = 76.0;
/// Room before the first dot and after the last.
pub const TRACK_PAD: f32 = 48.0;
/// How far inside either edge a selected dot is kept, clear of the fade mask.
const VISIBLE_PAD: f32 = 130.0;
/// Half the version card's width: its centre is kept at least this far from either edge.
const CARD_HALF: f32 = 158.0;
/// How far the card's arrow may lean off-centre to keep pointing at a dot near an edge.
const ARROW_LEAN: f32 = 130.0;

/// A horizontally panned track of `count` checkpoints seen through a `viewport` wide window.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Track {
    pub count: usize,
    pub viewport: f32,
    pub offset: f32,
}

impl Track {
    #[must_use]
    pub fn width(&self) -> f32 {
        TRACK_PAD * 2.0 + gaps(self.count) * TRACK_PITCH
    }

    #[must_use]
    pub fn dot_x(index: usize) -> f32 {
        TRACK_PAD + gaps(index + 1) * TRACK_PITCH
    }

    #[must_use]
    pub fn clamp(&self, offset: f32) -> f32 {
        offset.clamp(0.0, (self.width() - self.viewport).max(0.0))
    }

    /// Panned to the right end, where the present is.
    #[must_use]
    pub fn at_present(self) -> Self {
        Self {
            offset: self.clamp(f32::INFINITY),
            ..self
        }
    }

    #[must_use]
    pub fn panned_by(self, delta: f32) -> Self {
        Self {
            offset: self.clamp(self.offset + delta),
            ..self
        }
    }

    /// Pans just enough to keep dot `index` inside the unmasked middle of the viewport.
    #[must_use]
    pub fn revealing(self, index: usize) -> Self {
        let x = Self::dot_x(index);
        let offset = if x < self.offset + VISIBLE_PAD {
            self.clamp(x - VISIBLE_PAD)
        } else if x > self.offset + self.viewport - VISIBLE_PAD {
            self.clamp(x - self.viewport + VISIBLE_PAD)
        } else {
            self.offset
        };
        Self { offset, ..self }
    }

    /// Where dot `index` is on screen, from the viewport's left edge.
    #[must_use]
    pub fn screen_x(&self, index: usize) -> f32 {
        Self::dot_x(index) - self.offset
    }

    /// The version card's centre over dot `index`, and its arrow's lean from that centre: the
    /// card stays inside the panel and the arrow keeps pointing at the dot when it cannot.
    #[must_use]
    pub fn card(&self, index: usize) -> (f32, f32) {
        let dot = self.screen_x(index);
        let centre = dot.max(CARD_HALF).min(self.viewport - CARD_HALF);
        let lean = (dot - centre).clamp(-ARROW_LEAN, ARROW_LEAN);
        (centre, lean)
    }
}

/// `max(0, n - 1)` as a float. Checkpoint counts are capped at 500 per page, far inside `f32`.
fn gaps(count: usize) -> f32 {
    f32::from(u16::try_from(count.saturating_sub(1)).unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(count: usize) -> Track {
        Track {
            count,
            viewport: 400.0,
            offset: 0.0,
        }
    }

    #[test]
    fn a_short_track_never_pans() {
        let short = track(3).at_present();
        assert!((short.width() - 248.0).abs() < f32::EPSILON);
        assert!(short.offset.abs() < f32::EPSILON);
        assert!(short.panned_by(500.0).offset.abs() < f32::EPSILON);
    }

    #[test]
    fn opens_on_the_present() {
        let long = track(20).at_present();
        assert!((long.offset - (long.width() - 400.0)).abs() < f32::EPSILON);
        assert!((long.screen_x(19) - (400.0 - TRACK_PAD)).abs() < f32::EPSILON);
    }

    #[test]
    fn revealing_a_dot_keeps_it_clear_of_the_edges() {
        let long = track(20).at_present().revealing(0);
        assert!(long.offset.abs() < f32::EPSILON);
        let moved = track(20).revealing(10);
        assert!((moved.screen_x(10) - (400.0 - 130.0)).abs() < 0.001);
    }

    #[test]
    fn the_card_stays_inside_and_its_arrow_leans_to_the_dot() {
        let (centre, lean) = track(3).card(0);
        assert!((centre - CARD_HALF).abs() < f32::EPSILON);
        assert!((lean - (TRACK_PAD - CARD_HALF)).abs() < f32::EPSILON);
    }
}
