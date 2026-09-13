//! World-space geometry shared by the document and the canvas maths. Plain `f64`, no
//! dependencies; `peek-ui` converts to gpui pixels at its boundary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub fn distance_to(self, other: Self) -> f64 {
        (self - other).length()
    }

    #[must_use]
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    #[must_use]
    pub fn scaled(self, factor: f64) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }
}

impl std::ops::Add for Point {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for Point {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    #[must_use]
    pub fn scaled(self, factor: f64) -> Self {
        Self::new(self.width * factor, self.height * factor)
    }
}

/// An axis-aligned rectangle given by its top-left corner and size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// The rectangle spanning two arbitrary corners (handles inverted drags).
    #[must_use]
    pub fn from_corners(a: Point, b: Point) -> Self {
        let min = Point::new(a.x.min(b.x), a.y.min(b.y));
        let max = Point::new(a.x.max(b.x), a.y.max(b.y));
        Self::new(min, Size::new(max.x - min.x, max.y - min.y))
    }

    #[must_use]
    pub fn min(self) -> Point {
        self.origin
    }

    #[must_use]
    pub fn max(self) -> Point {
        Point::new(
            self.origin.x + self.size.width,
            self.origin.y + self.size.height,
        )
    }

    #[must_use]
    pub fn center(self) -> Point {
        Point::new(
            self.origin.x + self.size.width / 2.0,
            self.origin.y + self.size.height / 2.0,
        )
    }

    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        let max = self.max();
        point.x >= self.origin.x && point.x <= max.x && point.y >= self.origin.y && point.y <= max.y
    }

    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        let (a_max, b_max) = (self.max(), other.max());
        self.origin.x < b_max.x
            && other.origin.x < a_max.x
            && self.origin.y < b_max.y
            && other.origin.y < a_max.y
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let min = Point::new(
            self.origin.x.min(other.origin.x),
            self.origin.y.min(other.origin.y),
        );
        let (a_max, b_max) = (self.max(), other.max());
        let max = Point::new(a_max.x.max(b_max.x), a_max.y.max(b_max.y));
        Self::from_corners(min, max)
    }

    /// Grows the rectangle by `amount` on every side (negative shrinks).
    #[must_use]
    pub fn dilated(self, amount: f64) -> Self {
        Self::new(
            Point::new(self.origin.x - amount, self.origin.y - amount),
            Size::new(
                (self.size.width + 2.0 * amount).max(0.0),
                (self.size.height + 2.0 * amount).max(0.0),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_normalise_inverted_drags() {
        let rect = Rect::from_corners(Point::new(10.0, 10.0), Point::new(0.0, 5.0));
        assert_eq!(rect.origin, Point::new(0.0, 5.0));
        assert_eq!(rect.size, Size::new(10.0, 5.0));
    }

    #[test]
    fn intersection_is_strict_on_edges() {
        let a = Rect::new(Point::new(0.0, 0.0), Size::new(10.0, 10.0));
        let touching = Rect::new(Point::new(10.0, 0.0), Size::new(5.0, 5.0));
        let overlapping = Rect::new(Point::new(9.0, 9.0), Size::new(5.0, 5.0));
        assert!(!a.intersects(touching));
        assert!(a.intersects(overlapping));
        assert_eq!(a.union(touching).size, Size::new(15.0, 10.0));
    }
}
