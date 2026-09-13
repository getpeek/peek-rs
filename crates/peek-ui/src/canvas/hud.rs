//! Bottom-left zoom cluster, ported from `ZoomIndicator.tsx` and `.zoom-indicator`.
//!
//! Deliberately *not* the tool palette's surface: the reference makes this one a pill with a
//! transparent fill and a half-alpha hairline, so it reads as a readout the pointer can nudge
//! rather than a second panel of tools. Its buttons are 26 px with 14 px glyphs against the
//! palette's 30 / 16, and the percentage is a 44 px column of tabular figures.
//!
//! The camera lock is the one addition. The reference floats it bottom-right and shows it only
//! while locked, which leaves no visible way to lock; here it closes the cluster it governs, and
//! the controls it disables sit beside it.

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Icon, Selectable, Sizable, Size, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, App, ClickEvent, Context, Div, FocusHandle, FontWeight, Pixels, SharedString, Window,
    div, px, transparent_black,
};
use peek_theme::ActivePeekTheme;

use super::CanvasView;
use crate::commands::{self, actions};

/// `.zoom-indicator button`, and the 14 px glyphs inside them.
const BUTTON: Pixels = px(26.0);
const GLYPH: Pixels = px(14.0);
/// `.lvl`: wide enough that the percentage does not shuffle the buttons as it changes.
const READOUT: Pixels = px(44.0);

pub(super) fn render(view: &CanvasView, cx: &mut Context<CanvasView>) -> impl IntoElement {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "display only"
    )]
    let percent = (view.camera.zoom * 100.0).round() as u32;
    let locked = view.camera_locked;
    let focus = view.focus_handle.clone();

    pill(cx)
        .id("zoom-indicator")
        .test_support()
        .absolute()
        .bottom_4()
        .left_4()
        .child(step(
            "zoom-out",
            IconName::Minus,
            "Zoom out",
            &actions::zoom::Out,
            &focus,
            locked,
        ))
        .child(readout(percent, locked, &focus, cx))
        .child(step(
            "zoom-in",
            IconName::Plus,
            "Zoom in",
            &actions::zoom::In,
            &focus,
            locked,
        ))
        .child(step(
            "zoom-fit",
            IconName::Maximize,
            "Fit all nodes",
            &actions::zoom::FitView,
            &focus,
            locked,
        ))
        .child(camera_lock(locked, &focus))
}

/// `.zoom-indicator`: a transparent pill behind a half-alpha hairline, with a 1 px gap and 3 px of
/// padding. No shadow and no fill — it sits over the canvas rather than on it.
fn pill(cx: &App) -> Div {
    let theme = cx.peek_theme();
    div()
        .h_flex()
        .items_center()
        .gap(px(1.0))
        .p(px(3.0))
        .rounded(theme.radius_pill)
        .bg(transparent_black())
        .border_1()
        .border_color(theme.node_border.opacity(0.5))
        .text_color(theme.fg_muted)
}

/// The glyph is a child rather than `Button::icon`, which would size it to three quarters of the
/// frame and render a 19 px glyph in a 26 px button.
fn button(id: impl Into<SharedString>, icon: IconName) -> Button {
    Button::new(id.into())
        .ghost()
        .with_size(Size::Size(BUTTON))
        .size(BUTTON)
        .p_0()
        .child(Icon::new(icon).size(GLYPH))
}

/// A zoom control. Locking the camera disables them, as the reference does: the lock's whole
/// meaning is that pan and zoom are frozen, so leaving these live would contradict it.
fn step(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    action: &dyn Action,
    focus: &FocusHandle,
    locked: bool,
) -> Button {
    button(id, icon)
        .disabled(locked)
        .accessibility_label(tooltip)
        .tooltip_with_action(tooltip, action, Some(commands::CANVAS))
        .on_click(dispatch(action.boxed_clone(), focus))
}

/// `.lvl`: tabular figures in the full-strength text colour, and a click target of its own.
fn readout(percent: u32, locked: bool, focus: &FocusHandle, cx: &App) -> Button {
    Button::new("zoom-reset")
        .ghost()
        .with_size(Size::Size(BUTTON))
        .h(BUTTON)
        .min_w(READOUT)
        .p_0()
        .disabled(locked)
        .text_color(cx.peek_theme().fg)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(div().w_full().text_center().child(format!("{percent}%")))
        .tooltip_with_action("Reset zoom", &actions::zoom::Reset, Some(commands::CANVAS))
        .on_click(dispatch(Box::new(actions::zoom::Reset), focus))
}

/// Icon-only, so it carries both a tooltip and an explicit accessible name, and it stays visibly
/// selected while locked — the glyph alone should not be the only cue. Never disabled: it is the
/// way back out of the state it puts the cluster in.
fn camera_lock(locked: bool, focus: &FocusHandle) -> Button {
    let (icon, label) = if locked {
        (IconName::Lock, "Unlock camera")
    } else {
        (IconName::LockOpen, "Lock camera")
    };
    button("camera-lock", icon)
        .selected(locked)
        .accessibility_label(label)
        .tooltip_with_action(
            label,
            &actions::view::ToggleCameraLock,
            Some(commands::CANVAS),
        )
        .on_click(dispatch(Box::new(actions::view::ToggleCameraLock), focus))
}

/// Chrome dispatches through the canvas so buttons take the same path the keyboard does.
fn dispatch(
    action: Box<dyn Action>,
    focus: &FocusHandle,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let focus = focus.clone();
    move |_, window, cx| {
        focus.dispatch_action(&*action, window, cx);
    }
}
