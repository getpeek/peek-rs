//! Freehand stroke tessellation: a port of `perfect-freehand` 1.2.3, the library
//! `~/labs/peek/src/canvas/nodes/Draw/DrawNode.tsx` renders draw nodes with.
//!
//! [`outline`] is `getStroke`: it turns recorded `[x, y, pressure]` samples into the closed
//! polygon that surrounds them. [`path`] is `getSvgPathFromStroke`, minus the string: it pairs
//! the polygon's vertices into the quadratic segments the renderer draws.
//!
//! Only the options Peek passes are ported. The upstream taper, flat-cap and easing options
//! have no caller here, and dropping them removes the branches they guard.

use std::f64::consts::PI;

type Vec2 = [f64; 2];

/// How fast simulated pressure follows the pen's speed.
const RATE_OF_PRESSURE_CHANGE: f64 = 0.275;
/// Half a turn, nudged off exactly `PI` so renderers do not drop the degenerate arc.
const FIXED_PI: f64 = PI + 0.0001;
const START_CAP_SEGMENTS: f64 = 13.0;
const END_CAP_SEGMENTS: f64 = 29.0;
const CORNER_CAP_SEGMENTS: f64 = 13.0;
/// Samples this close to the end of the stroke are jitter from lifting the pen.
const END_NOISE_THRESHOLD: f64 = 3.0;
const MIN_STREAMLINE_T: f64 = 0.15;
const STREAMLINE_T_RANGE: f64 = 0.85;
const MIN_RADIUS: f64 = 0.01;
/// Drawn lines almost always start slow, so the first sample starts thin.
const DEFAULT_FIRST_PRESSURE: f64 = 0.25;
const DEFAULT_PRESSURE: f64 = 0.5;
const UNIT_OFFSET: Vec2 = [1.0, 1.0];

/// The knobs `DrawNode.tsx` sets. [`Default`] is `perfect-freehand`'s own default, which is
/// what Peek uses for everything but `size`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeOptions {
    /// Base diameter of the stroke. Peek passes `stroke_width * 4`.
    pub size: f64,
    /// How much pressure narrows the stroke. `0` ignores pressure entirely.
    pub thinning: f64,
    /// How far apart consecutive outline points must be before one is kept.
    pub smoothing: f64,
    /// How far each sample is pulled toward the one before it.
    pub streamline: f64,
    /// Derive pressure from the distance between samples instead of reading it.
    pub simulate_pressure: bool,
}

impl Default for StrokeOptions {
    fn default() -> Self {
        Self {
            size: 16.0,
            thinning: 0.5,
            smoothing: 0.5,
            streamline: 0.5,
            simulate_pressure: true,
        }
    }
}

/// A closed outline as quadratic segments: start at [`StrokePath::start`], then curve through
/// every `(control, end)` pair.
#[derive(Debug, Clone, PartialEq)]
pub struct StrokePath {
    pub start: [f64; 2],
    pub segments: Vec<([f64; 2], [f64; 2])>,
}

/// The closed polygon surrounding `points`, each an `[x, y, pressure]` sample.
#[must_use]
pub fn outline(points: &[[f64; 3]], options: StrokeOptions) -> Vec<[f64; 2]> {
    trace(&stroke_points(points, options), options)
}

/// The outline as quadratic segments, each running to a vertex through the midpoint of the
/// edge that leads to it.
///
/// `getSvgPathFromStroke` emits one control point more than it emits endpoints, leaving a
/// trailing half-segment that browsers discard; the shape closes over that last sliver either
/// way, so this drops it rather than reproducing malformed output.
#[must_use]
pub fn path(outline: &[[f64; 2]]) -> Option<StrokePath> {
    let &start = outline.first()?;
    let segments = outline
        .windows(2)
        .map(|edge| (midpoint(edge[0], edge[1]), edge[1]))
        .collect();
    Some(StrokePath { start, segments })
}

fn negate(a: Vec2) -> Vec2 {
    [-a[0], -a[1]]
}

fn add(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn subtract(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(a: Vec2, factor: f64) -> Vec2 {
    [a[0] * factor, a[1] * factor]
}

/// Rotate a quarter turn clockwise.
fn perpendicular(a: Vec2) -> Vec2 {
    [a[1], -a[0]]
}

fn dot(a: Vec2, b: Vec2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn length(a: Vec2) -> f64 {
    a[0].hypot(a[1])
}

fn unit(a: Vec2) -> Vec2 {
    scale(a, 1.0 / length(a))
}

fn distance(a: Vec2, b: Vec2) -> f64 {
    (a[1] - b[1]).hypot(a[0] - b[0])
}

fn distance_squared(a: Vec2, b: Vec2) -> f64 {
    let offset = subtract(a, b);
    offset[0] * offset[0] + offset[1] * offset[1]
}

fn midpoint(a: Vec2, b: Vec2) -> Vec2 {
    [f64::midpoint(a[0], b[0]), f64::midpoint(a[1], b[1])]
}

fn lerp(a: Vec2, b: Vec2, t: f64) -> Vec2 {
    add(a, scale(subtract(b, a), t))
}

/// Move `from` along `direction` by `amount`.
fn project(from: Vec2, direction: Vec2, amount: f64) -> Vec2 {
    add(from, scale(direction, amount))
}

fn rotate_around(a: Vec2, center: Vec2, radians: f64) -> Vec2 {
    let (sin, cos) = radians.sin_cos();
    let offset = subtract(a, center);
    add(
        [
            offset[0] * cos - offset[1] * sin,
            offset[0] * sin + offset[1] * cos,
        ],
        center,
    )
}

fn stroke_radius(size: f64, thinning: f64, pressure: f64) -> f64 {
    size * (0.5 - thinning * (0.5 - pressure))
}

/// Pressure from velocity: the further the pen travelled since the last sample, the lighter it
/// was pressing.
fn simulate_pressure(previous: f64, travelled: f64, size: f64) -> f64 {
    let speed = (travelled / size).min(1.0);
    let rate = (1.0 - speed).min(1.0);
    (previous + (rate - previous) * (speed * RATE_OF_PRESSURE_CHANGE)).min(1.0)
}

/// A recorded sample. Pressure is absent for the points `input_points` synthesises, which then
/// take [`DEFAULT_PRESSURE`] the way an undefined third element does upstream.
#[derive(Debug, Clone, Copy)]
struct InputPoint {
    point: Vec2,
    pressure: Option<f64>,
}

/// A sample after streamlining, carrying the direction and arc length the outline needs.
#[derive(Debug, Clone, Copy)]
struct StrokePoint {
    point: Vec2,
    pressure: f64,
    vector: Vec2,
    distance: f64,
    running_length: f64,
}

fn valid_pressure(pressure: f64) -> Option<f64> {
    (pressure >= 0.0).then_some(pressure)
}

/// Pads the degenerate inputs: a two-sample stroke would otherwise render as a dash, and a
/// one-sample stroke has no direction at all.
fn input_points(points: &[[f64; 3]]) -> Vec<InputPoint> {
    let mut points: Vec<InputPoint> = points
        .iter()
        .map(|&[x, y, pressure]| InputPoint {
            point: [x, y],
            pressure: valid_pressure(pressure),
        })
        .collect();

    if points.len() == 2 {
        let (first, last) = (points[0].point, points[1].point);
        points.truncate(1);
        for step in 1..5 {
            points.push(InputPoint {
                point: lerp(first, last, f64::from(step) / 4.0),
                pressure: None,
            });
        }
    }

    if points.len() == 1 {
        points.push(InputPoint {
            point: add(points[0].point, UNIT_OFFSET),
            pressure: points[0].pressure,
        });
    }

    points
}

/// `getStrokePoints`: streamline the samples and annotate each with its direction, the step
/// that reached it and the arc length so far.
fn stroke_points(points: &[[f64; 3]], options: StrokeOptions) -> Vec<StrokePoint> {
    if points.is_empty() {
        return Vec::new();
    }

    let interpolation = MIN_STREAMLINE_T + (1.0 - options.streamline) * STREAMLINE_T_RANGE;
    let points = input_points(points);
    let last_index = points.len() - 1;

    let mut previous = StrokePoint {
        point: points[0].point,
        pressure: points[0].pressure.unwrap_or(DEFAULT_FIRST_PRESSURE),
        vector: UNIT_OFFSET,
        distance: 0.0,
        running_length: 0.0,
    };
    let mut result = vec![previous];
    let mut running_length = 0.0;
    let mut reached_minimum_length = false;

    for (index, candidate) in points.iter().enumerate().skip(1) {
        let point = lerp(previous.point, candidate.point, interpolation);
        let step = distance(point, previous.point);
        if step <= 0.0 {
            continue;
        }
        running_length += step;

        // The first samples of a stroke are pen-down jitter until the line has some length.
        let too_short =
            index < last_index && !reached_minimum_length && running_length < options.size;
        if too_short {
            continue;
        }
        reached_minimum_length = true;

        previous = StrokePoint {
            point,
            pressure: candidate.pressure.unwrap_or(DEFAULT_PRESSURE),
            vector: unit(subtract(previous.point, point)),
            distance: step,
            running_length,
        };
        result.push(previous);
    }

    result[0].vector = result.get(1).map_or([0.0, 0.0], |second| second.vector);
    result
}

/// Averaging the opening samples stops a stroke from starting fat.
fn initial_pressure(points: &[StrokePoint], options: StrokeOptions) -> f64 {
    let Some(first) = points.first() else {
        return DEFAULT_PRESSURE;
    };
    points
        .iter()
        .take(10)
        .fold(first.pressure, |accumulated, current| {
            let pressure = if options.simulate_pressure {
                simulate_pressure(accumulated, current.distance, options.size)
            } else {
                current.pressure
            };
            f64::midpoint(accumulated, pressure)
        })
}

/// The radius at one sample, and the pressure it came from. A zero `thinning` opts out of
/// pressure entirely, matching upstream's falsy check.
fn radius_at(current: &StrokePoint, previous_pressure: f64, options: StrokeOptions) -> (f64, f64) {
    if options.thinning.abs() < f64::EPSILON {
        return (options.size / 2.0, current.pressure);
    }
    let pressure = if options.simulate_pressure {
        simulate_pressure(previous_pressure, current.distance, options.size)
    } else {
        current.pressure
    };
    (
        stroke_radius(options.size, options.thinning, pressure),
        pressure,
    )
}

/// The two sides of the outline as they are walked, plus the carry-over the next sample reads.
#[derive(Debug)]
struct Sides {
    left: Vec<Vec2>,
    right: Vec<Vec2>,
    previous_left: Vec2,
    previous_right: Vec2,
    previous_pressure: f64,
    previous_vector: Vec2,
    /// Set after a corner cap so the same corner is not capped twice.
    after_sharp_corner: bool,
    radius: f64,
    first_radius: Option<f64>,
    minimum_distance: f64,
}

impl Sides {
    /// A turn sharper than a right angle: cap the corner instead of projecting across it.
    fn push_corner_cap(&mut self, center: Vec2, radius: f64) {
        let offset = scale(perpendicular(self.previous_vector), radius);
        let step = 1.0 / CORNER_CAP_SEGMENTS;
        let mut turn = 0.0;
        while turn <= 1.0 {
            let left = rotate_around(subtract(center, offset), center, FIXED_PI * turn);
            let right = rotate_around(add(center, offset), center, FIXED_PI * -turn);
            self.left.push(left);
            self.right.push(right);
            self.previous_left = left;
            self.previous_right = right;
            turn += step;
        }
    }

    /// Keep a projected pair only once it has cleared the smoothing distance; `always` forces
    /// the opening samples in so the start cap has something to anchor to.
    fn push_pair(&mut self, center: Vec2, offset: Vec2, always: bool) {
        let left = subtract(center, offset);
        if always || distance_squared(self.previous_left, left) > self.minimum_distance {
            self.left.push(left);
            self.previous_left = left;
        }
        let right = add(center, offset);
        if always || distance_squared(self.previous_right, right) > self.minimum_distance {
            self.right.push(right);
            self.previous_right = right;
        }
    }
}

/// `getStrokeOutlinePoints`: walk the samples, collecting the outline's left and right sides,
/// then join them through the end cap and back around the start cap.
fn trace(points: &[StrokePoint], options: StrokeOptions) -> Vec<Vec2> {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return Vec::new();
    };
    if options.size <= 0.0 {
        return Vec::new();
    }

    let total_length = last.running_length;
    let mut sides = Sides {
        left: Vec::new(),
        right: Vec::new(),
        previous_left: first.point,
        previous_right: first.point,
        previous_pressure: initial_pressure(points, options),
        previous_vector: first.vector,
        after_sharp_corner: false,
        radius: stroke_radius(options.size, options.thinning, last.pressure),
        first_radius: None,
        minimum_distance: (options.size * options.smoothing).powi(2),
    };

    for (index, current) in points.iter().enumerate() {
        let is_last = index == points.len() - 1;
        if !is_last && total_length - current.running_length < END_NOISE_THRESHOLD {
            continue;
        }

        let (radius, pressure) = radius_at(current, sides.previous_pressure, options);
        sides.first_radius = sides.first_radius.or(Some(radius));
        sides.radius = radius.max(MIN_RADIUS);

        let next_vector = if is_last {
            current.vector
        } else {
            points[index + 1].vector
        };
        let next_dot = if is_last {
            1.0
        } else {
            dot(current.vector, next_vector)
        };
        let sharp_here =
            dot(current.vector, sides.previous_vector) < 0.0 && !sides.after_sharp_corner;
        let sharp_next = next_dot < 0.0;

        if sharp_here || sharp_next {
            sides.push_corner_cap(current.point, sides.radius);
            sides.after_sharp_corner = sharp_next;
            continue;
        }
        sides.after_sharp_corner = false;

        if is_last {
            let offset = scale(perpendicular(current.vector), sides.radius);
            sides.left.push(subtract(current.point, offset));
            sides.right.push(add(current.point, offset));
            continue;
        }

        let direction = perpendicular(lerp(next_vector, current.vector, next_dot));
        sides.push_pair(current.point, scale(direction, sides.radius), index <= 1);

        sides.previous_pressure = pressure;
        sides.previous_vector = current.vector;
    }

    if points.len() == 1 {
        return dot_outline(first.point, sides.first_radius.unwrap_or(sides.radius));
    }

    let start_cap = sides
        .right
        .first()
        .map(|&right| round_start_cap(first.point, right))
        .unwrap_or_default();
    let end_cap = round_end_cap(last.point, perpendicular(negate(last.vector)), sides.radius);

    let mut result = sides.left;
    result.extend(end_cap);
    sides.right.reverse();
    result.extend(sides.right);
    result.extend(start_cap);
    result
}

/// A stroke too short to have direction is a circle.
fn dot_outline(center: Vec2, radius: f64) -> Vec<Vec2> {
    let away = unit(perpendicular(subtract(center, add(center, UNIT_OFFSET))));
    let start = project(center, away, -radius);
    let step = 1.0 / START_CAP_SEGMENTS;
    let mut points = Vec::new();
    let mut turn = step;
    while turn <= 1.0 {
        points.push(rotate_around(start, center, FIXED_PI * 2.0 * turn));
        turn += step;
    }
    points
}

/// Half a turn from the right side of the stroke back to the left, closing the outline.
fn round_start_cap(center: Vec2, right: Vec2) -> Vec<Vec2> {
    let step = 1.0 / START_CAP_SEGMENTS;
    let mut points = Vec::new();
    let mut turn = step;
    while turn <= 1.0 {
        points.push(rotate_around(right, center, FIXED_PI * turn));
        turn += step;
    }
    points
}

/// One and a half turns, so a stroke that doubles back on itself still ends in a round tip.
fn round_end_cap(center: Vec2, direction: Vec2, radius: f64) -> Vec<Vec2> {
    let start = project(center, direction, radius);
    let step = 1.0 / END_CAP_SEGMENTS;
    let mut points = Vec::new();
    let mut turn = step;
    while turn < 1.0 {
        points.push(rotate_around(start, center, FIXED_PI * 3.0 * turn));
        turn += step;
    }
    points
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the path conversion is exact arithmetic over the outline it is given"
)]
mod tests {
    use super::{StrokeOptions, outline, path};

    /// The options `DrawNode.tsx` renders with, for a node's default stroke width of 4.
    fn peek_options() -> StrokeOptions {
        StrokeOptions {
            size: 4.0 * 4.0,
            ..StrokeOptions::default()
        }
    }

    fn bounds(outline: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
        outline.iter().fold(
            ([f64::MAX, f64::MAX], [f64::MIN, f64::MIN]),
            |(min, max), point| {
                (
                    [min[0].min(point[0]), min[1].min(point[1])],
                    [max[0].max(point[0]), max[1].max(point[1])],
                )
            },
        )
    }

    /// Half the outline's extent across the stroke's direction of travel.
    fn half_width(outline: &[[f64; 2]]) -> f64 {
        let (min, max) = bounds(outline);
        (max[1] - min[1]) / 2.0
    }

    #[test]
    fn an_empty_stroke_has_no_outline() {
        assert!(outline(&[], peek_options()).is_empty());
        assert_eq!(path(&[]), None);
    }

    #[test]
    fn a_single_point_still_draws_something() {
        let points = [[10.0, 10.0, 0.5]];
        let stroke = outline(&points, peek_options());

        assert!(!stroke.is_empty());
        let (min, max) = bounds(&stroke);
        // The synthesised second sample is one unit away, so the whole mark stays within a
        // stroke width of where the pen went down.
        assert!(min[0] > 10.0 - 16.0 && min[1] > 10.0 - 16.0, "{min:?}");
        assert!(max[0] < 10.0 + 17.0 && max[1] < 10.0 + 17.0, "{max:?}");
    }

    #[test]
    fn a_straight_stroke_is_a_capsule_around_its_line() {
        let points = [[0.0, 50.0, 0.5], [200.0, 50.0, 0.5]];
        let stroke = outline(&points, peek_options());

        let (min, max) = bounds(&stroke);
        // Half a stroke width of cap either end, and no more.
        assert!(min[0] > -9.0 && max[0] < 209.0, "{min:?} {max:?}");
        assert!(min[1] > 41.0 && max[1] < 59.0, "{min:?} {max:?}");
        // Both sides of the line are traced, not just one.
        assert!(stroke.iter().any(|point| point[1] < 49.0));
        assert!(stroke.iter().any(|point| point[1] > 51.0));
    }

    #[test]
    fn pressure_widens_the_stroke() {
        let options = StrokeOptions {
            simulate_pressure: false,
            ..peek_options()
        };
        let light: Vec<[f64; 3]> = (0..20)
            .map(|step| [f64::from(step) * 10.0, 50.0, 0.1])
            .collect();
        let heavy: Vec<[f64; 3]> = (0..20)
            .map(|step| [f64::from(step) * 10.0, 50.0, 0.9])
            .collect();

        assert!(half_width(&outline(&light, options)) < half_width(&outline(&heavy, options)));
    }

    #[test]
    fn thinning_off_ignores_pressure() {
        let options = StrokeOptions {
            simulate_pressure: false,
            thinning: 0.0,
            ..peek_options()
        };
        let light: Vec<[f64; 3]> = (0..20)
            .map(|step| [f64::from(step) * 10.0, 50.0, 0.1])
            .collect();
        let heavy: Vec<[f64; 3]> = (0..20)
            .map(|step| [f64::from(step) * 10.0, 50.0, 0.9])
            .collect();

        assert_eq!(outline(&light, options), outline(&heavy, options));
    }

    #[test]
    fn streamline_pulls_the_stroke_away_from_its_corners() {
        let zigzag: Vec<[f64; 3]> = (0..12)
            .map(|step| {
                let x = f64::from(step) * 40.0;
                let y = if step % 2 == 0 { 0.0 } else { 120.0 };
                [x, y, 0.5]
            })
            .collect();
        let raw = StrokeOptions {
            streamline: 0.0,
            ..peek_options()
        };
        let smoothed = StrokeOptions {
            streamline: 1.0,
            ..peek_options()
        };

        let (raw_min, raw_max) = bounds(&outline(&zigzag, raw));
        let (smooth_min, smooth_max) = bounds(&outline(&zigzag, smoothed));

        assert!(smooth_max[1] - smooth_min[1] < raw_max[1] - raw_min[1]);
        assert!(smooth_max[0] - smooth_min[0] < raw_max[0] - raw_min[0]);
    }

    #[test]
    fn the_path_curves_through_every_edge_midpoint() {
        let points = [[0.0, 50.0, 0.5], [200.0, 50.0, 0.5]];
        let stroke = outline(&points, peek_options());
        let path = path(&stroke).expect("a non-empty stroke has a path");

        assert_eq!(path.start, stroke[0]);
        assert_eq!(path.segments.len(), stroke.len() - 1);
        for (index, (control, end)) in path.segments.iter().enumerate() {
            let (from, to) = (stroke[index], stroke[index + 1]);
            let expected = [f64::midpoint(from[0], to[0]), f64::midpoint(from[1], to[1])];
            assert_eq!(*control, expected);
            assert_eq!(*end, to);
        }
    }
}
