//! Drawing one transcript message.
//!
//! Each kind gets the treatment `~/labs/peek/src/canvas/nodes/Agent/MessageItem.tsx` gives it:
//! a conversational turn is a role line over prose, and everything else — context, thoughts,
//! system notes — is a compact chip that reads as machinery rather than as speech.

use gpui_kit::assets::IconName;
use gpui_kit::component::marker::Marker;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, HighlightStyle, Hsla, SharedString, StyleRefinement, Window, div, px, rems,
};
use peek_document::{AgentMessage, NodeType};
use peek_theme::ActivePeekTheme;

use crate::node::scaled;

use super::plan_block;

/// `.message-content h1` … `h4` in the reference's `agent.css`, as fractions of the rem the node
/// chrome is designed against.
fn heading_rems(level: u8) -> f32 {
    match level {
        1 => 1.0,
        2 => 0.906_25,
        3 => 0.8125,
        _ => 0.75,
    }
}

/// The markdown prose style: `.message-content` and the rules under it.
///
/// Everything gpui-base takes in rems follows the camera on its own. Two things do not — it
/// derives a heading's size from a pixel base, and a code fence from the theme's mono size — and
/// those are the ones that render a heading at full size inside a node drawn at a third of it.
fn markdown_style(window: &Window, cx: &App) -> TextViewStyle {
    let theme = cx.peek_theme();
    let rem = window.rem_size();
    let code_block = StyleRefinement::default()
        .px(rems(0.75))
        .py(rems(0.5))
        .text_size(rems(0.75))
        .bg(theme.node_inset)
        .border_1()
        .border_color(theme.node_border)
        .rounded(scaled(theme.radius_card));

    TextViewStyle::default()
        .paragraph_gap(rems(0.625))
        .heading_font_size(move |level, _| px(heading_rems(level) * f32::from(rem)))
        .code_block(code_block)
        .inline_code(HighlightStyle {
            color: Some(theme.accent_soft),
            background_color: Some(theme.node_inset),
            ..HighlightStyle::default()
        })
}

/// A conversational turn: a role line, then the message as markdown.
pub(super) fn turn(
    id: SharedString,
    message: &AgentMessage,
    window: &Window,
    cx: &App,
) -> AnyElement {
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
        .child(div().text_color(theme.fg).child(
            TextView::markdown(id, message.message.clone()).style(markdown_style(window, cx)),
        ))
        .into_any_element()
}

/// The assistant's dot carries the node's accent and a soft glow, so a long transcript still
/// reads at a glance as alternating speakers.
fn role_dot(color: Hsla, glow: bool, cx: &App) -> impl IntoElement {
    div()
        .size(rems(0.375))
        .rounded_full()
        .bg(color)
        // The one place a pixel constant survives the camera: gpui box shadows take
        // `Pixels`, with no rem-relative form to scale the glow with the zoom.
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
                .size(rems(1.5))
                .flex_shrink_0()
                .rounded(rems(0.4375))
                .bg(background)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon).size(rems(0.875)).text_color(tint)),
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
    window: &Window,
    cx: &App,
) -> AnyElement {
    match message.kind.as_str() {
        "plan" => plan_block::render(message.plan_entries.as_deref().unwrap_or_default(), cx),
        "context" | "thought" | "system" => chip(message, context_updated, cx),
        _ => turn(id, message, window, cx),
    }
}
