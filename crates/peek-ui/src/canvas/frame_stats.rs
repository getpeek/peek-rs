//! Frame timing for the canvas, off unless `PEEK_FRAME_STATS=1`.
//!
//! Exists because the render path has no other observable: everything downstream of
//! [`super::CanvasView::render`] is rebuilt every frame, so "is this change faster" is not a
//! question the tests can answer. Samples are kept per phase and reported as a mean and a p95,
//! because the mean alone hides exactly the stalls that read as dropped frames.

use std::time::Duration;

/// How many frames are collected before a line is logged.
const REPORT_EVERY: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Render,
    Prepaint,
    Paint,
}

#[derive(Debug, Default)]
struct Samples {
    micros: Vec<u128>,
}

impl Samples {
    fn push(&mut self, elapsed: Duration) {
        self.micros.push(elapsed.as_micros());
    }

    /// Mean and p95 in milliseconds. Sorts in place; the buffer is cleared right after.
    fn summary(&mut self) -> (f64, f64) {
        if self.micros.is_empty() {
            return (0.0, 0.0);
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "a frame time in microseconds is far inside f64"
        )]
        let mean = self.micros.iter().sum::<u128>() as f64 / self.micros.len() as f64;
        self.micros.sort_unstable();
        let index = (self.micros.len() * 95) / 100;
        #[allow(
            clippy::cast_precision_loss,
            reason = "a frame time in microseconds is far inside f64"
        )]
        let p95 = self.micros[index.min(self.micros.len() - 1)] as f64;
        (mean / 1000.0, p95 / 1000.0)
    }
}

/// Per-phase frame timings, reported every [`REPORT_EVERY`] frames.
///
/// Disabled builds keep a zero-sized-ish struct and every entry point returns immediately, so
/// leaving the calls in costs one boolean test per frame.
#[derive(Debug)]
pub(crate) struct FrameStats {
    enabled: bool,
    frames: usize,
    render: Samples,
    prepaint: Samples,
    paint: Samples,
    visible_nodes: usize,
    total_nodes: usize,
}

impl FrameStats {
    pub(crate) fn new() -> Self {
        Self {
            enabled: std::env::var("PEEK_FRAME_STATS").is_ok_and(|value| value == "1"),
            frames: 0,
            render: Samples::default(),
            prepaint: Samples::default(),
            paint: Samples::default(),
            visible_nodes: 0,
            total_nodes: 0,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn record(&mut self, phase: Phase, elapsed: Duration) {
        if !self.enabled {
            return;
        }
        match phase {
            Phase::Render => self.render.push(elapsed),
            Phase::Prepaint => self.prepaint.push(elapsed),
            Phase::Paint => self.paint.push(elapsed),
        }
    }

    /// Counts the frame and logs a line every [`REPORT_EVERY`]. Called once per render, after
    /// the node list is known.
    pub(crate) fn tick(&mut self, visible_nodes: usize, total_nodes: usize) {
        if !self.enabled {
            return;
        }
        self.visible_nodes = visible_nodes;
        self.total_nodes = total_nodes;
        self.frames += 1;
        if self.frames < REPORT_EVERY {
            return;
        }
        let (render_mean, render_p95) = self.render.summary();
        let (prepaint_mean, prepaint_p95) = self.prepaint.summary();
        let (paint_mean, paint_p95) = self.paint.summary();
        log::info!(
            "frame stats over {} frames, {} of {} nodes visible: \
             render {render_mean:.2}/{render_p95:.2} ms, \
             prepaint {prepaint_mean:.2}/{prepaint_p95:.2} ms, \
             paint {paint_mean:.2}/{paint_p95:.2} ms (mean/p95)",
            self.frames,
            self.visible_nodes,
            self.total_nodes,
        );
        self.frames = 0;
        self.render.micros.clear();
        self.prepaint.micros.clear();
        self.paint.micros.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{Duration, Samples};

    #[test]
    fn p95_picks_the_tail_not_the_mean() {
        let mut samples = Samples::default();
        for _ in 0..99 {
            samples.push(Duration::from_millis(1));
        }
        samples.push(Duration::from_millis(50));
        let (mean, p95) = samples.summary();
        assert!(mean < 2.0, "one stall must not dominate the mean: {mean}");
        assert!(p95 >= 1.0, "p95 must sit at or above the body: {p95}");
    }

    #[test]
    fn an_empty_window_reports_zero_rather_than_panicking() {
        assert_eq!(Samples::default().summary(), (0.0, 0.0));
    }
}
