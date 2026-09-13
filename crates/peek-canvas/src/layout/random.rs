//! d3's `lcg`, ported constant for constant.
//!
//! d3-force is deterministic: every simulation shares one linear congruential generator seeded
//! at 1, and the only thing it is used for is nudging two coincident nodes apart. Reproducing
//! it exactly is what lets the layout be unit-tested at all — the same document always settles
//! in the same place.

const MULTIPLIER: u64 = 1_664_525;
const INCREMENT: u64 = 1_013_904_223;
const MODULUS: u64 = 4_294_967_296;

/// The nudge applied to a zero separation, `(random() - 0.5) * 1e-6` in `jiggle.js`.
const JIGGLE_SCALE: f64 = 1e-6;

#[derive(Debug, Clone)]
pub(super) struct Lcg {
    state: u64,
}

impl Default for Lcg {
    fn default() -> Self {
        Self { state: 1 }
    }
}

impl Lcg {
    #[allow(
        clippy::cast_precision_loss,
        reason = "the state is below 2^32 and converts exactly"
    )]
    fn next(&mut self) -> f64 {
        self.state = (MULTIPLIER * self.state + INCREMENT) % MODULUS;
        self.state as f64 / MODULUS as f64
    }

    pub(super) fn jiggle(&mut self) -> f64 {
        (self.next() - 0.5) * JIGGLE_SCALE
    }
}

#[cfg(test)]
mod tests {
    use super::Lcg;

    #[test]
    fn jiggle_is_tiny_and_reproducible() {
        let mut generator = Lcg::default();
        let stream: Vec<f64> = (0..5).map(|_| generator.jiggle()).collect();
        let mut again = Lcg::default();
        let repeat: Vec<f64> = (0..5).map(|_| again.jiggle()).collect();

        assert_eq!(stream, repeat, "a fresh generator replays the same stream");
        assert!(
            stream.iter().all(|value| value.abs() <= 5e-7),
            "and stays a nudge"
        );
        assert!((stream[0] - stream[1]).abs() > 0.0, "the stream advances");
    }
}
