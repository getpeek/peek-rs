//! The message box and the one button beside it.

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::{Disableable, Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Focusable, Window, div, rems};
use peek_theme::ActivePeekTheme;

use crate::node::{TextInsetExt, scaled};

use super::view::AgentView;

pub(super) fn render(
    view: &AgentView,
    window: &mut Window,
    cx: &mut Context<AgentView>,
) -> AnyElement {
    let theme = cx.peek_theme();
    let loading = view.is_loading();
    let empty = view.composer().read(cx).value().trim().is_empty();

    div()
        .id(gpui_kit::SharedString::from(format!(
            "{}-composer",
            view.node()
        )))
        .test_support()
        .h_flex()
        .items_end()
        .gap(rems(0.45))
        .w_full()
        .p(rems(0.5))
        .border_t_1()
        .border_color(theme.node_border)
        .bg(theme.node_bg)
        // The canvas resolves presses in world space and its listeners run first, so the
        // textarea never sees the click that should focus it.
        .on_click({
            let composer = view.composer().clone();
            move |_, window, cx| {
                window.focus(&composer.focus_handle(cx), cx);
            }
        })
        .child(
            // The box is drawn here rather than by the textarea, whose own frame would sit a
            // fixed number of screen pixels away from the text and paint outside itself once
            // `scaled_text_inset` pulls the frame back onto the camera.
            div()
                .flex_1()
                .min_w_0()
                .bg(theme.node_inset)
                .border_1()
                .border_color(theme.node_border)
                .rounded(scaled(theme.radius_card))
                .child(
                    Textarea::new(view.composer())
                        .disabled(loading)
                        .appearance(false)
                        .bordered(false)
                        .text_size(rems(0.78125))
                        .scaled_text_inset(window),
                ),
        )
        .child(button(loading, empty, window, cx))
        .into_any_element()
}

/// One button with two jobs, as the reference has it: send while idle, stop while a turn runs.
/// Stop is never disabled — a turn that will not end is exactly when it is needed.
fn button(
    loading: bool,
    empty: bool,
    _window: &mut Window,
    cx: &mut Context<AgentView>,
) -> impl IntoElement {
    let theme = cx.peek_theme();
    let (icon, tooltip) = if loading {
        (IconName::Square, "Stop")
    } else {
        (IconName::SendHorizontal, "Send message")
    };

    Button::new("agent-send")
        .primary()
        .when(loading, ButtonVariants::danger)
        .disabled(!loading && empty)
        .size(rems(1.875))
        .tooltip(tooltip)
        .child(Icon::new(icon).size(rems(1.0)))
        .on_click(cx.listener(move |view, _, window, cx| {
            if loading {
                view.stop(cx);
            } else {
                view.send(window, cx);
            }
        }))
        .text_color(theme.fg)
}
