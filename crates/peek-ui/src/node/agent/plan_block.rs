//! The agent's plan, as a checklist.
//!
//! A plan arrives repeatedly through the turn and replaces itself in place, so this is the one
//! block a reader watches change. `priority` is on the wire but the reference never shows it.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, div, rems};
use peek_document::PlanEntry;
use peek_theme::ActivePeekTheme;

use crate::node::scaled;

pub(super) fn render(entries: &[PlanEntry], cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    if entries.is_empty() {
        return div().into_any_element();
    }

    div()
        .v_flex()
        .gap(rems(0.4))
        .p(rems(0.6))
        .rounded(scaled(theme.radius_card))
        .bg(theme.node_inset)
        .border_1()
        .border_color(theme.node_border)
        .child(
            div()
                .text_size(rems(0.6))
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_color(theme.fg_subtle)
                .child("PLAN"),
        )
        .children(entries.iter().map(|entry| row(entry, cx)))
        .into_any_element()
}

fn row(entry: &PlanEntry, cx: &App) -> impl IntoElement {
    let theme = cx.peek_theme();
    let (icon, tint, text, struck) = match entry.status.as_str() {
        "completed" => (IconName::CircleCheck, theme.green, theme.fg_subtle, true),
        "in_progress" => (IconName::CircleDot, theme.accent, theme.fg, false),
        _ => (IconName::Circle, theme.fg_muted, theme.fg_muted, false),
    };

    div()
        .h_flex()
        .items_start()
        .gap(rems(0.4))
        .child(Icon::new(icon).size(rems(0.875)).text_color(tint))
        .child(
            div()
                .text_size(rems(0.72))
                .text_color(text)
                .when(struck, gpui_kit::Styled::line_through)
                .child(entry.content.clone()),
        )
}
