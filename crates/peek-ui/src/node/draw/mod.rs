//! The Draw node: `~/labs/peek/src/canvas/nodes/Draw/DrawNode.tsx`.
//!
//! A freehand stroke, not a card: no header, no resizer, no frame — just the filled path, the
//! way `DrawNode.tsx` renders a bare `<svg><path/></svg>`. The outline is tessellated by
//! [`peek_canvas::stroke`] and painted through gpui's path API, which has no `d` string but
//! takes the same quadratic segments the SVG would.

pub(crate) mod color;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ContentMask, FillOptions, FillRule, Hsla, Path, PathBuilder, PathStyle,
    Pixels, Point, canvas, point, px,
};
use peek_canvas::stroke::{self, StrokeOptions, StrokePath};
use peek_document::{DrawData, Node};
use peek_theme::{ActivePeekTheme, PeekTheme};

use super::kind::NodeContext;

/// `DrawNode.tsx` hands `getStroke` four times the stored width.
pub(crate) const SIZE_PER_STROKE_WIDTH: f64 = 4.0;

pub(crate) fn title(data: &DrawData) -> String {
    match data.points.len() {
        1 => "1 point".to_string(),
        count => format!("{count} points"),
    }
}

pub(crate) fn body(
    node: &Node,
    data: &DrawData,
    context: NodeContext<'_>,
    cx: &mut App,
) -> AnyElement {
    let fill = ink(&data.color, context.selected, cx.peek_theme());
    let options = StrokeOptions {
        size: data.stroke_width * SIZE_PER_STROKE_WIDTH,
        ..StrokeOptions::default()
    };
    let outline = stroke::path(&stroke::outline(&data.points, options));
    let width = node.size().width;

    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            // Rem scaling does not reach a path's coordinates, so the stroke reads the zoom
            // back off the element: the canvas lays every node out at `world * zoom`.
            let placement = Placement {
                origin: bounds.origin,
                scale: f64::from(f32::from(bounds.size.width)) / width,
            };
            let Some(path) = outline
                .as_ref()
                .and_then(|outline| tessellate(outline, placement))
            else {
                return;
            };
            // The SVG clips to its viewport, so a stroke never escapes a resized node.
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.paint_path(path, fill);
            });
        },
    )
    .size_full()
    .into_any_element()
}

/// A drawing has no border to thicken and no header to tint, so selection has to recolour the
/// ink itself. `DrawNode.tsx` hardcodes `#7dd3fc` for this — the only occurrence in the whole
/// reference, and the one selection affordance in it that ignores the theme, so the stroke
/// takes the colour the canvas already rings selected nodes with instead.
fn ink(color: &str, selected: bool, theme: &PeekTheme) -> Hsla {
    if selected {
        return theme.selection;
    }
    color::resolve(color, theme)
}

/// Where a stroke's points land on screen. The committed node scales its world-unit points by
/// the zoom it reads off its own bounds; the live preview is already in screen units and passes
/// `scale: 1.0`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Placement {
    pub(crate) origin: Point<Pixels>,
    pub(crate) scale: f64,
}

impl Placement {
    /// Points are stored relative to the node's origin; paths are painted in window space.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "document geometry is f64 and gpui paints in f32 pixels"
    )]
    fn place(self, at: [f64; 2]) -> Point<Pixels> {
        point(
            self.origin.x + px((at[0] * self.scale) as f32),
            self.origin.y + px((at[1] * self.scale) as f32),
        )
    }
}

/// Fills with the nonzero rule: an outline that crosses itself at a sharp corner is one solid
/// stroke in SVG, and lyon's even-odd default would punch a hole through the overlap.
pub(crate) fn tessellate(outline: &StrokePath, placement: Placement) -> Option<Path<Pixels>> {
    if !placement.scale.is_finite() || placement.scale <= 0.0 {
        return None;
    }
    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::default().with_fill_rule(FillRule::NonZero),
    ));
    builder.move_to(placement.place(outline.start));
    for &(control, end) in &outline.segments {
        builder.curve_to(placement.place(end), placement.place(control));
    }
    builder.close();
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Pixels, point, px, white};
    use peek_canvas::stroke::{self, StrokeOptions};
    use peek_config::ThemeId;
    use peek_theme::{PeekTheme, builtin};

    use super::{Placement, ink, tessellate};

    /// A short arc, the shape a quick flick of the pen leaves. `useDrawTool.ts` insets the
    /// points it records, so they sit well inside the node it commits them to.
    fn arc() -> Vec<[f64; 3]> {
        (0..24)
            .map(|step| {
                let angle = f64::from(step) * 0.12;
                [60.0 + angle.cos() * 40.0, 60.0 - angle.sin() * 40.0, 0.5]
            })
            .collect()
    }

    #[test]
    fn a_stroke_tessellates_into_triangles_where_the_node_is() {
        let outline = stroke::path(&stroke::outline(&arc(), StrokeOptions::default()))
            .expect("a recorded stroke has a path");
        let placement = Placement {
            origin: point(px(500.0), px(300.0)),
            scale: 2.0,
        };

        let path = tessellate(&outline, placement).expect("lyon fills a closed outline");

        assert!(!path.vertices.is_empty(), "the fill has geometry");
        assert_eq!(
            path.vertices.len() % 3,
            0,
            "a filled path is whole triangles"
        );
        // Placed and scaled about the node's own origin, not the window's.
        let left = path
            .vertices
            .iter()
            .fold(Pixels::MAX, |left, vertex| left.min(vertex.xy_position.x));
        assert!(
            left >= px(500.0) && left < px(540.0),
            "the stroke sits just inside the node's left edge: {left:?}"
        );
    }

    #[test]
    fn selecting_a_drawing_recolours_its_ink() {
        let theme = PeekTheme::from_spec(builtin::spec(ThemeId::Midday));

        assert_eq!(ink("white", false, &theme), white(), "the stored colour");
        assert_eq!(
            ink("white", true, &theme),
            theme.selection,
            "selection wins over the document's colour, since there is no border to thicken"
        );
        assert_eq!(
            ink("var(--pk-fg)", false, &theme),
            theme.fg,
            "a stroke drawn in a theme colour follows the theme"
        );
    }

    #[test]
    fn a_collapsed_node_paints_nothing() {
        let outline = stroke::path(&stroke::outline(&arc(), StrokeOptions::default()))
            .expect("a recorded stroke has a path");
        let placement = Placement {
            origin: point(px(0.0), px(0.0)),
            scale: 0.0,
        };

        assert!(tessellate(&outline, placement).is_none());
    }
}
