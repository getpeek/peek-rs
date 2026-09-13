//! The jump-mode overlay: a scrim over the canvas and one badge per labelled node. The port of
//! `JumpLabels.tsx` and `jump.css`.
//!
//! Badges sit outside the camera's rem scope, so they keep a constant screen size at every
//! zoom — the label is chrome for reaching a node, not part of it.

use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, MouseButton, MouseDownEvent, div, px};
use peek_canvas::{Camera, JumpMode, JumpTarget};
use peek_theme::ActivePeekTheme;

use super::{CanvasView, screen_point};
use crate::commands::actions;

/// `jump.css` anchors a badge at its node's top-left and pulls it back over the corner with
/// `translate(-35%, -35%)`. gpui has no self-relative transform on a div and the badge's width
/// follows its label, so the shift is a fixed approximation of that fraction.
const BADGE_NUDGE: f32 = -7.0;

pub(super) fn render(
    jump: &JumpMode,
    camera: Camera,
    cx: &mut Context<CanvasView>,
) -> impl IntoElement {
    let theme = cx.peek_theme().clone();
    let badges: Vec<_> = jump
        .targets()
        .iter()
        .map(|target| badge(target, jump, camera, cx))
        .collect();

    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(theme.bg.opacity(0.45))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _: &MouseDownEvent, window, cx| {
                window.dispatch_action(Box::new(actions::tool::Select), cx);
            }),
        )
        .children(badges)
}

fn badge(
    target: &JumpTarget,
    jump: &JumpMode,
    camera: Camera,
    cx: &App,
) -> impl IntoElement + use<> {
    let theme = cx.peek_theme();
    let origin = screen_point(camera, target.world);
    let matching = jump.matches(target);
    // The typed prefix recedes and the remainder carries the accent, so the keys still to
    // press are the ones that stand out.
    let (typed, rest) = target
        .label
        .split_at(if matching { jump.typed().len() } else { 0 });

    div()
        .absolute()
        .left(origin.x + px(BADGE_NUDGE))
        .top(origin.y + px(BADGE_NUDGE))
        .h_flex()
        .px_1()
        .rounded(theme.radius_pill)
        .bg(theme.bg)
        .border_1()
        .border_color(if matching {
            theme.node_border
        } else {
            theme.node_border.opacity(0.5)
        })
        .when(matching, gpui_kit::Styled::shadow_md)
        .when(!matching, |this| this.opacity(0.5))
        .text_xs()
        .font_semibold()
        .child(
            div()
                .text_color(theme.fg_subtle)
                .child(typed.to_uppercase()),
        )
        .child(
            div()
                .text_color(if matching {
                    theme.accent
                } else {
                    theme.fg_muted
                })
                .child(rest.to_uppercase()),
        )
}
