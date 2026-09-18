//! What the node shows with nothing to say yet.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, div, rems};
use peek_theme::ActivePeekTheme;

/// A configured node with no conversation.
pub(super) fn waiting(cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    centred()
        .child(
            div()
                .text_size(rems(0.875))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(theme.fg)
                .child("Ask questions about your dataset"),
        )
        .child(
            div()
                .text_size(rems(0.75))
                .italic()
                .text_color(theme.fg_subtle)
                .child("Get insights and analysis from your data"),
        )
        .into_any_element()
}

/// Nothing in `settings.json` to talk to. The composer is hidden rather than disabled: there is
/// nowhere for a message to go, and an inert box invites typing into it.
pub(super) fn unconfigured(cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    centred()
        .child(
            Icon::new(IconName::TriangleAlert)
                .size(rems(1.375))
                .text_color(theme.red),
        )
        .child(
            div()
                .text_size(rems(0.8125))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(theme.fg)
                .child("No AI backend configured"),
        )
        .child(
            div()
                .max_w(rems(17.5))
                .text_center()
                .text_size(rems(0.75))
                .text_color(theme.fg_muted)
                .child("Add an `ai.ollama` or `ai.acp` block to ~/peek/settings.json to use the agent."),
        )
        .into_any_element()
}

fn centred() -> gpui_kit::Div {
    div()
        .v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap(rems(0.5))
}
