//! The canvas-space boxes drawn around every region — the port of `RegionHalos.tsx` and
//! `wayfinding.css`'s `.wf-region-halo`.
//!
//! These are painted **over** the node elements, not under them. In the reference the layer is
//! a React Flow `ViewportPortal`, which mounts into `.react-flow__viewport-portal` — the last
//! child of the viewport, after the node renderer. It has to be: a confirmed halo is a veil of
//! the *canvas background colour*, and a background-coloured pool painted underneath the cards
//! would do nothing at all. Every halo is pointer-transparent, so hit-testing is untouched.

use gpui_kit::{
    BorderStyle, Bounds, Hsla, Pixels, Point, Window, point, px, quad, size, transparent_black,
};
use peek_canvas::{Camera, Rect};

use crate::canvas::screen_rect;

/// `.wf-region-halo`'s `border-radius`, in world units.
const RADIUS: f32 = 28.0;
/// `.wf-region-halo.suggested`'s dashed hairline and its two alpha mixes.
const SUGGESTED_BORDER: f32 = 1.5;
const SUGGESTED_BORDER_ALPHA: f32 = 0.55;
const SUGGESTED_FILL_ALPHA: f32 = 0.04;
/// `.wf-region-flash`: a solid ring over a faint wash, fading out over 900 ms.
const FLASH_BORDER: f32 = 2.0;
const FLASH_FILL_ALPHA: f32 = 0.07;

/// How many bands approximate the confirmed halo's radial pool. gpui has no radial gradient
/// (`gpui-pre`'s `Background` is solid, linear, slash or checkerboard), so the CSS's
/// `radial-gradient(ellipse 80% 80% …)` is rebuilt as concentric rounded quads shrinking toward
/// the centre. Twelve is enough that the steps sit under what the eye resolves at the zoom a
/// halo is visible at, and it is twelve quads per region per frame, only while zoomed out.
const POOL_BANDS: usize = 12;
/// The CSS's three stops: alpha at the centre, at 46% out, and where it reaches nothing.
const POOL_CENTER_ALPHA: f32 = 0.94;
const POOL_MID_ALPHA: f32 = 0.60;
const POOL_MID_STOP: f32 = 0.46;
const POOL_EDGE_STOP: f32 = 0.74;

/// One region's box, ready to paint. Built by the view because only it can derive the members'
/// bounds; `alpha` is already the cross-fade, so this file never looks at the camera's zoom.
pub(crate) struct Halo {
    pub world: Rect,
    /// The region's own palette colour, for the suggested border and the flash ring.
    pub color: Hsla,
    /// The canvas background the confirmed pool dissolves into.
    pub background: Hsla,
    pub kind: HaloKind,
}

pub(crate) enum HaloKind {
    /// The Lasso Glow, at `cross_fade(zoom)`. Never built at all when that is zero.
    Confirmed { alpha: f32 },
    /// Dashed and always visible: it is asking for a decision.
    Suggested,
    /// A one-shot ring marking the region a selection just folded into, at `alpha`.
    ///
    /// Its own box rather than a state on the confirmed halo, whose styling is fully
    /// transparent at the zoom where folding happens.
    Flash { alpha: f32 },
}

pub(crate) fn paint(bounds: Bounds<Pixels>, camera: Camera, halos: &[Halo], window: &mut Window) {
    for halo in halos {
        let screen = offset(screen_rect(camera, halo.world), bounds.origin);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a world radius scaled into screen pixels"
        )]
        let radius = px(RADIUS * camera.zoom as f32);
        match halo.kind {
            HaloKind::Confirmed { alpha } => pool(screen, halo.background, alpha, window),
            HaloKind::Suggested => window.paint_quad(quad(
                screen,
                radius,
                halo.color.opacity(SUGGESTED_FILL_ALPHA),
                px(SUGGESTED_BORDER),
                halo.color.opacity(SUGGESTED_BORDER_ALPHA),
                BorderStyle::Dashed,
            )),
            HaloKind::Flash { alpha } => window.paint_quad(quad(
                screen,
                radius,
                halo.color.opacity(FLASH_FILL_ALPHA * alpha),
                px(FLASH_BORDER),
                halo.color.opacity(alpha),
                BorderStyle::Solid,
            )),
        }
    }
}

/// The radial pool, as concentric rounded quads from the outside in.
///
/// Each band is a scaled copy of the box about its centre, so the shape of the pool follows the
/// shape of the region the way `ellipse 80% 80%` does — a uniform inset would collapse the
/// inner bands of a wide region to nothing. Bands are painted over one another, so what the eye
/// sees at a point is the sum of every band covering it; the per-band alpha is therefore the
/// *difference* between neighbouring stops rather than the stop itself, or a dozen overlapping
/// washes would reach opaque long before the centre.
fn pool(screen: Bounds<Pixels>, background: Hsla, alpha: f32, window: &mut Window) {
    if alpha <= 0.0 {
        return;
    }
    let center = screen.center();
    let mut painted = 0.0;
    for band in (0..POOL_BANDS).rev() {
        // Spread across the visible part of the gradient only: past POOL_EDGE_STOP the CSS is
        // already fully transparent, so bands out there would be a third of the budget spent
        // painting nothing.
        #[allow(clippy::cast_precision_loss, reason = "a band index under twenty")]
        let distance = POOL_EDGE_STOP * (band + 1) as f32 / POOL_BANDS as f32;
        let target = stop_alpha(distance);
        // What this band has to add on top of everything already painted outside it.
        let added = if painted >= 1.0 {
            0.0
        } else {
            ((target - painted) / (1.0 - painted)).max(0.0)
        };
        painted = target;
        if added <= 0.0 {
            continue;
        }
        let band_size = size(screen.size.width * distance, screen.size.height * distance);
        let band_bounds = Bounds::new(
            center - point(band_size.width / 2.0, band_size.height / 2.0),
            band_size,
        );
        window.paint_quad(quad(
            band_bounds,
            band_size.width.min(band_size.height) / 2.0,
            background.opacity(added * alpha),
            px(0.0),
            transparent_black(),
            BorderStyle::Solid,
        ));
    }
}

/// The CSS's own stops, linearly interpolated: 94% at the centre, 60% at 46% out, nothing from
/// 74% out. `distance` is 0 at the centre and 1 at the box's edge.
fn stop_alpha(distance: f32) -> f32 {
    if distance >= POOL_EDGE_STOP {
        return 0.0;
    }
    if distance <= POOL_MID_STOP {
        let t = distance / POOL_MID_STOP;
        return POOL_CENTER_ALPHA + (POOL_MID_ALPHA - POOL_CENTER_ALPHA) * t;
    }
    let t = (distance - POOL_MID_STOP) / (POOL_EDGE_STOP - POOL_MID_STOP);
    POOL_MID_ALPHA * (1.0 - t)
}

fn offset(bounds: Bounds<Pixels>, by: Point<Pixels>) -> Bounds<Pixels> {
    Bounds::new(bounds.origin + by, bounds.size)
}

#[cfg(test)]
mod tests {
    use super::{POOL_EDGE_STOP, stop_alpha};

    #[test]
    fn the_pool_is_densest_at_its_centre_and_gone_at_the_edge() {
        assert!((stop_alpha(0.0) - 0.94).abs() < 1e-6);
        assert!((stop_alpha(0.46) - 0.60).abs() < 1e-6);
        assert!((stop_alpha(POOL_EDGE_STOP) - 0.0).abs() < f32::EPSILON);
        assert!((stop_alpha(1.0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_pool_never_gets_denser_further_out() {
        let mut previous = f32::MAX;
        for step in 0..=100 {
            #[allow(clippy::cast_precision_loss, reason = "a loop counter under a hundred")]
            let alpha = stop_alpha(step as f32 / 100.0);
            assert!(alpha <= previous + 1e-6, "rose again at {step}");
            previous = alpha;
        }
    }
}
