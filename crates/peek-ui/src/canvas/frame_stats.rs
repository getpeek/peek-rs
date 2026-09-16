//! Frame timing for the canvas: per-phase logging under `PEEK_FRAME_STATS=1`, and a wall-clock
//! frame rate under `--fps`.
//!
//! Exists because the render path has no other observable: everything downstream of
//! [`super::CanvasView::render`] is rebuilt every frame, so "is this change faster" is not a
//! question the tests can answer. Samples are kept per phase and reported as a mean and a p95,
//! because the mean alone hides exactly the stalls that read as dropped frames.
//!
//! The two are independent switches over one counter. Phase logging answers "where did the time
//! go" after the fact; the frame rate answers "is it smooth" while you are dragging something.
//! Either may be on alone.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How many frames are collected before a line is logged.
const REPORT_EVERY: usize = 60;

/// Frames are counted over this window, so the reading tracks the gesture in progress rather
/// than averaging it against the whole session.
const FPS_WINDOW: Duration = Duration::from_secs(1);

/// No frame for this long and the canvas is idle, not slow. gpui redraws on demand: a still
/// canvas draws nothing at all, and reporting that as 0 fps would read as a stall.
pub(super) const IDLE_AFTER: Duration = Duration::from_millis(400);

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

/// What the frame-rate readout shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reading {
    pub(crate) fps: f64,
    /// The slowest interval in the window. A mean hides exactly the stalls that read as a
    /// dropped frame, which is the whole reason the readout exists.
    pub(crate) worst_ms: f64,
}

/// Per-phase frame timings, reported every [`REPORT_EVERY`] frames, and the wall-clock frame
/// rate behind `--fps`.
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
    fps: bool,
    /// When each of the last [`FPS_WINDOW`] worth of frames was counted, oldest first.
    beats: VecDeque<Instant>,
}

impl FrameStats {
    pub(crate) fn new(fps: bool) -> Self {
        Self {
            enabled: std::env::var("PEEK_FRAME_STATS").is_ok_and(|value| value == "1"),
            frames: 0,
            render: Samples::default(),
            prepaint: Samples::default(),
            paint: Samples::default(),
            visible_nodes: 0,
            total_nodes: 0,
            fps,
            beats: VecDeque::new(),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn fps_enabled(&self) -> bool {
        self.fps
    }

    /// The frame rate over the last [`FPS_WINDOW`], or `None` when the canvas has gone idle or
    /// has not yet drawn twice — one timestamp measures no interval.
    pub(crate) fn reading(&self, now: Instant) -> Option<Reading> {
        let newest = *self.beats.back()?;
        if now.saturating_duration_since(newest) >= IDLE_AFTER {
            return None;
        }
        let oldest = *self.beats.front()?;
        let span = newest.saturating_duration_since(oldest);
        if self.beats.len() < 2 || span.is_zero() {
            return None;
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "a frame count inside a one-second window is far inside f64"
        )]
        let intervals = (self.beats.len() - 1) as f64;
        let worst = self
            .beats
            .iter()
            .zip(self.beats.iter().skip(1))
            .map(|(before, after)| after.saturating_duration_since(*before))
            .max()
            .unwrap_or_default();
        Some(Reading {
            fps: intervals / span.as_secs_f64(),
            worst_ms: worst.as_secs_f64() * 1000.0,
        })
    }

    /// Records that a frame happened, dropping the samples that have aged out of the window.
    fn beat(&mut self, now: Instant) {
        self.beats.push_back(now);
        while self
            .beats
            .front()
            .is_some_and(|oldest| now.saturating_duration_since(*oldest) > FPS_WINDOW)
        {
            self.beats.pop_front();
        }
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
        if self.fps {
            self.beat(Instant::now());
        }
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
    use super::{Duration, FrameStats, Instant, Samples};

    /// A counter with `n` frames laid down `gap` apart, ending exactly `now`.
    fn beating(gaps: &[Duration]) -> (FrameStats, Instant) {
        let mut stats = FrameStats::new(true);
        let start = Instant::now();
        let mut at = start;
        stats.beat(at);
        for gap in gaps {
            at += *gap;
            stats.beat(at);
        }
        (stats, at)
    }

    #[test]
    fn sixty_frames_a_sixtieth_apart_read_as_sixty() {
        let gaps = vec![Duration::from_micros(16_667); 59];
        let (stats, now) = beating(&gaps);
        let reading = stats.reading(now).expect("the canvas is drawing");
        assert!(
            (reading.fps - 60.0).abs() < 1.0,
            "expected about 60 fps, got {}",
            reading.fps
        );
    }

    /// The point of the readout: one stall has to be visible even though the mean absorbs it.
    #[test]
    fn a_single_stall_shows_in_the_worst_interval_without_moving_the_rate_much() {
        let mut gaps = vec![Duration::from_micros(16_667); 58];
        gaps.push(Duration::from_millis(50));
        let (stats, now) = beating(&gaps);
        let reading = stats.reading(now).expect("the canvas is drawing");
        assert!(reading.worst_ms >= 50.0, "{}", reading.worst_ms);
        assert!(
            reading.fps > 45.0,
            "one stall must not collapse the rate: {}",
            reading.fps
        );
    }

    #[test]
    fn a_still_canvas_reads_as_idle_rather_than_as_zero() {
        let (stats, now) = beating(&[Duration::from_micros(16_667); 10]);
        assert!(stats.reading(now).is_some());
        assert_eq!(stats.reading(now + super::IDLE_AFTER), None);
    }

    /// One timestamp measures no interval, so there is nothing honest to report from it.
    #[test]
    fn a_single_frame_reports_nothing() {
        let (stats, now) = beating(&[]);
        assert_eq!(stats.reading(now), None);
    }

    #[test]
    fn frames_older_than_the_window_are_dropped() {
        let gaps = vec![Duration::from_millis(100); 30];
        let (stats, _) = beating(&gaps);
        assert!(
            stats.beats.len() <= 12,
            "a one-second window holds about eleven frames at 100 ms: {}",
            stats.beats.len()
        );
    }

    /// The wall clock is only sampled behind the flag; an unflagged run keeps an empty ring.
    #[test]
    fn an_unflagged_counter_samples_nothing() {
        let mut stats = FrameStats::new(false);
        stats.tick(1, 1);
        assert!(stats.beats.is_empty());
        assert_eq!(stats.reading(Instant::now()), None);
    }

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
