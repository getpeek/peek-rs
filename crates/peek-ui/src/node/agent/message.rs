//! Drawing one transcript message.
//!
//! Each kind gets the treatment `~/labs/peek/src/canvas/nodes/Agent/MessageItem.tsx` gives it:
//! a conversational turn is a role line over prose, and everything else — context, thoughts,
//! system notes — is a compact chip that reads as machinery rather than as speech.

use gpui_kit::assets::IconName;
use gpui_kit::component::marker::Marker;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Hsla, SharedString, div, px, rems};
use peek_document::{AgentMessage, NodeType};
use peek_theme::ActivePeekTheme;

use super::plan_block;

/// A conversational turn: a role line, then the message as markdown.
pub(super) fn turn(id: SharedString, message: &AgentMessage, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    let assistant = !message.is("user");
    let (dot, label) = if assistant {
        (theme.node_type(Some(NodeType::Agent)), "Assistant")
    } else {
        (theme.fg_subtle, "User")
    };

    div()
        .v_flex()
        .gap(rems(0.45))
        .child(
            div()
                .h_flex()
                .items_center()
                .gap(rems(0.4))
                .child(role_dot(dot, assistant, cx))
                .child(
                    div()
                        .text_size(rems(0.65))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(if assistant {
                            theme.accent_soft
                        } else {
                            theme.fg_muted
                        })
                        .child(label.to_uppercase()),
                ),
        )
        .child(
            div()
                .text_color(theme.fg)
                .child(TextView::markdown(id, message.message.clone())),
        )
        .into_any_element()
}

/// The assistant's dot carries the node's accent and a soft glow, so a long transcript still
/// reads at a glance as alternating speakers.
fn role_dot(color: Hsla, glow: bool, cx: &App) -> impl IntoElement {
    div()
        .size(px(6.0))
        .rounded_full()
        .bg(color)
        .when(glow, |this| {
            this.shadow(vec![gpui_kit::BoxShadow {
                color: cx.peek_theme().accent_line,
                offset: gpui_kit::point(px(0.0), px(0.0)),
                blur_radius: px(8.0),
                spread_radius: px(0.0),
                inset: false,
            }])
        })
}

/// A chip: machinery the conversation did, not something anyone said.
pub(super) fn chip(message: &AgentMessage, context_updated: bool, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    let (icon, tint, background, title, subtitle) = match message.kind.as_str() {
        "context" => (
            IconName::DatabaseZap,
            theme.green,
            theme.green_soft,
            if context_updated {
                "Context updated"
            } else {
                "Context inserted"
            }
            .to_string(),
            Some("Query and result".to_string()),
        ),
        "thought" => (
            IconName::Lightbulb,
            theme.fg_subtle,
            theme.node_inset,
            "Thinking".to_string(),
            None,
        ),
        _ => (
            IconName::Bot,
            theme.blue,
            theme.blue_soft,
            message.message.clone(),
            Some("Just for you".to_string()),
        ),
    };

    let body = (message.is("thought")).then(|| {
        div()
            .text_size(rems(0.72))
            .italic()
            .text_color(theme.fg_muted)
            .child(message.message.clone())
    });

    Marker::new()
        .child(
            div()
                .size(px(24.0))
                .flex_shrink_0()
                .rounded(theme.radius_card)
                .bg(background)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon).size(px(14.0)).text_color(tint)),
        )
        .child(
            div()
                .v_flex()
                .gap(rems(0.15))
                .child(
                    div()
                        .text_size(rems(0.72))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(theme.fg)
                        .child(title),
                )
                .children(subtitle.map(|text| {
                    div()
                        .text_size(rems(0.62))
                        .text_color(theme.fg_subtle)
                        .child(text)
                }))
                .children(body),
        )
        .into_any_element()
}

/// The row a transcript message renders as, whichever kind it is.
pub(super) fn render(
    id: SharedString,
    message: &AgentMessage,
    context_updated: bool,
    cx: &App,
) -> AnyElement {
    match message.kind.as_str() {
        "plan" => plan_block::render(message.plan_entries.as_deref().unwrap_or_default(), cx),
        "context" | "thought" | "system" => chip(message, context_updated, cx),
        _ => turn(id, message, cx),
    }
}
