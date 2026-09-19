//! Edge peekers — the port of `EdgePeekers.tsx` and `.wf-peeker`.
//!
//! A peeker is a transient compass: a region that has left the viewport pins a label to the
//! nearest edge, pointed at where it actually is, while the camera is moving and for a moment
//! after it settles. Hovering one holds the layer up so it stays clickable.

use gpui_kit::TestSupportExt;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{
    ClickEvent, Context, FontWeight, MouseButton, MouseDownEvent, Pixels, SharedString, div, px,
};
use peek_canvas::regions::crossfade::{INTERACTIVE_OPACITY, peek_opacity};
use peek_canvas::regions::derive::Derived;
use peek_canvas::regions::peeker::{self, Align, PEEKER_WIDTH, Placement};
use peek_document::Node;
use peek_theme::ActivePeekTheme;

use super::Frame;
use crate::canvas::CanvasView;
use crate::canvas::convert::pixels;

const NAME_SIZE: Pixels = px(17.0);
const COUNT_SIZE: Pixels = px(10.0);
const DESC_SIZE: Pixels = px(11.5);
/// The arrow is a glyph rather than the reference's rotated SVG triangle: gpui has no
/// self-relative rotation on a div, and eight compass points read the direction as well as a
/// continuous angle does at this size.
const ARROW_SIZE: Pixels = px(11.0);

pub(super) fn render(
    view: &CanvasView,
    regions: &[Derived],
    nodes: &[Node],
    frame: Frame,
    t: f64,
    cx: &mut Context<CanvasView>,
) -> Option<impl IntoElement> {
    let base = peek_opacity(t);
    if base <= 0.0 || !view.peekers_showing() {
        return None;
    }
    let placed: Vec<_> = regions
        .iter()
        .filter_map(|region| {
            peeker::place(region.bbox(nodes), frame.camera, frame.pane)
                .map(|placement| (region, placement))
        })
        .collect();
    if placed.is_empty() {
        return None;
    }

    let interactive = base > INTERACTIVE_OPACITY;
    #[allow(clippy::cast_possible_truncation, reason = "an opacity in 0..=1")]
    let opacity = base as f32;
    Some(
        div().absolute().inset_0().opacity(opacity).children(
            placed
                .into_iter()
                .map(|(region, placement)| peeker(region, placement, interactive, cx)),
        ),
    )
}

fn peeker(
    region: &Derived,
    placement: Placement,
    interactive: bool,
    cx: &mut Context<CanvasView>,
) -> impl IntoElement + use<> {
    let theme = cx.peek_theme().clone();
    let color = theme.region(region.color_index as usize);
    let right = placement.align == Align::Right;

    let row = div()
        .h_flex()
        .items_center()
        .gap(px(8.0))
        .when(right, gpui_kit::Styled::flex_row_reverse)
        .child(
            div()
                .flex_shrink_0()
                .text_size(ARROW_SIZE)
                .text_color(color)
                .child(arrow(placement.angle)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(NAME_SIZE)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg)
                .child(region.name.clone()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(COUNT_SIZE)
                .font_family("Monaspace Krypton")
                .text_color(theme.fg_subtle)
                .child(region.member_ids.len().to_string()),
        );

    let card = div()
        .id(SharedString::from(format!("peeker-{}", region.id)))
        .test_support()
        .absolute()
        .left(pixels(placement.origin.x))
        .top(pixels(placement.origin.y))
        .w(pixels(PEEKER_WIDTH))
        .v_flex()
        .gap(px(2.0))
        .p(px(6.0))
        .rounded(px(10.0))
        .when(right, |this| this.items_end().text_right())
        .child(row)
        .when(!region.desc.is_empty(), |this| {
            this.child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_size(DESC_SIZE)
                    .text_color(theme.fg_muted)
                    .child(region.desc.clone()),
            )
        });

    if !interactive {
        return card;
    }
    let id = region.id.clone();
    card.cursor_pointer()
        .hover(move |style| style.bg(theme.fg.opacity(0.06)))
        // Hovering holds the whole layer up: reaching for a label that is fading is otherwise a
        // race the pointer loses.
        .on_mouse_move(cx.listener(|view, _, _, cx| view.nudge_peekers(cx)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()),
        )
        .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
            view.fly_to_region(&id, window, cx);
        }))
}

/// The nearest of eight compass glyphs to `degrees` clockwise from east.
fn arrow(degrees: f64) -> &'static str {
    const POINTS: [&str; 8] = [
        "\u{2192}", "\u{2198}", "\u{2193}", "\u{2199}", "\u{2190}", "\u{2196}", "\u{2191}",
        "\u{2197}",
    ];
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "an index into an eight-entry table"
    )]
    let index = (((degrees / 45.0).round() as i64).rem_euclid(8)) as usize;
    POINTS[index]
}

#[cfg(test)]
mod tests {
    use super::arrow;

    #[test]
    fn the_glyph_follows_the_angle_round_the_compass() {
        assert_eq!(arrow(0.0), "\u{2192}");
        assert_eq!(arrow(90.0), "\u{2193}");
        assert_eq!(arrow(180.0), "\u{2190}");
        assert_eq!(arrow(-90.0), "\u{2191}");
        assert_eq!(
            arrow(359.0),
            "\u{2192}",
            "wraps rather than falling off the end"
        );
    }
}
