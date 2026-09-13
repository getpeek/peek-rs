//! The agent asking to run something.
//!
//! Deliberately not modal: it sits between the transcript and the composer, so the conversation
//! stays readable while the question is open — the reference makes the same choice, and a modal
//! over a canvas node would be a modal over the whole window.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, SharedString, div, rems};
use peek_acp::PermissionRequest;
use peek_theme::ActivePeekTheme;

use crate::node::scaled;

use super::acp;
use super::view::AgentView;

pub(super) fn render(request: &PermissionRequest, cx: &mut Context<AgentView>) -> AnyElement {
    let theme = cx.peek_theme();
    let options: Vec<(SharedString, SharedString, bool)> = request
        .options
        .iter()
        .map(|option| {
            (
                SharedString::from(option.id.clone()),
                SharedString::from(option.name.clone()),
                option.kind.starts_with("allow"),
            )
        })
        .collect();

    div()
        .v_flex()
        .gap(rems(0.5))
        .mx(rems(0.5))
        .mb(rems(0.4))
        .p(rems(0.6))
        .rounded(scaled(theme.radius_card))
        .bg(theme.node_inset)
        .border_1()
        .border_color(theme.accent_line)
        .child(
            div()
                .h_flex()
                .items_center()
                .gap(rems(0.35))
                .child(
                    Icon::new(IconName::ShieldAlert)
                        .size(rems(0.9375))
                        .text_color(theme.accent_soft),
                )
                .child(
                    div()
                        .text_size(rems(0.6))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(theme.accent_soft)
                        .child("PERMISSION NEEDED"),
                ),
        )
        .child(
            div()
                .text_size(rems(0.75))
                .text_color(theme.fg)
                .child(request.tool_title.clone()),
        )
        .child(
            div()
                .h_flex()
                .flex_wrap()
                .gap(rems(0.35))
                .children(options.into_iter().map(|(id, label, allow)| {
                    Button::new(SharedString::from(format!("permission-{id}")))
                        .when(allow, ButtonVariants::primary)
                        .when(!allow, ButtonVariants::ghost)
                        .label(label)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            acp::answer_permission(view, Some(id.to_string()), cx);
                        }))
                }))
                // Always offered, and the only way out when the agent sends no options at all.
                .child(
                    Button::new("permission-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(cx.listener(|view, _, _, cx| {
                            acp::answer_permission(view, None, cx);
                        })),
                ),
        )
        .into_any_element()
}
