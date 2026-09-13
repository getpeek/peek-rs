//! The one place `peek-canvas`'s `f64` geometry meets gpui's `f32` pixels.

#![allow(
    clippy::cast_possible_truncation,
    reason = "world/screen conversions are inherently lossy at the gpui boundary"
)]

use gpui_kit::{Bounds, Pixels, px};
use peek_canvas::{Point, Rect, Size};

pub(crate) fn pixels(value: f64) -> Pixels {
    px(value as f32)
}

pub(crate) fn to_pixel_point(point: Point) -> gpui_kit::Point<Pixels> {
    gpui_kit::point(px(point.x as f32), px(point.y as f32))
}

pub(crate) fn to_pixel_size(size: Size) -> gpui_kit::Size<Pixels> {
    gpui_kit::size(px(size.width as f32), px(size.height as f32))
}

pub(crate) fn to_pixel_bounds(rect: Rect) -> Bounds<Pixels> {
    Bounds::new(to_pixel_point(rect.origin), to_pixel_size(rect.size))
}

pub(crate) fn from_pixel_point(point: gpui_kit::Point<Pixels>) -> Point {
    Point::new(f64::from(f32::from(point.x)), f64::from(f32::from(point.y)))
}

pub(crate) fn from_pixel_size(size: gpui_kit::Size<Pixels>) -> Size {
    Size::new(
        f64::from(f32::from(size.width)),
        f64::from(f32::from(size.height)),
    )
}
