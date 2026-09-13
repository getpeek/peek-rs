//! Tool calls, as disclosures.
//!
//! Two shapes reach the same widget: an ACP `acp_tool` message, which carries its own live
//! status, and an Ollama `tool_call` paired with the `tool_result` that answered it. Both
//! collapse to one row, because a transcript full of expanded JSON is unreadable.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::collapsible::Collapsible;
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Hsla, SharedString, div, rems};
use peek_document::AgentMessage;
use peek_theme::ActivePeekTheme;

use crate::node::scaled;

use super::rows::ToolBlock;

/// An ACP tool: title, kind, and a status that changes while the turn runs.
pub(super) fn acp(
    message: &AgentMessage,
    open: bool,
    loading: bool,
    cx: &App,
) -> (SharedString, AnyElement) {
    let id = SharedString::from(message.tool_call_id.clone().unwrap_or_default());
    let status = message.tool_status.as_deref().unwrap_or("pending");
    let failed = status == "failed" || message.is_error == Some(true);

    let body = (!message.message.trim().is_empty())
        .then(|| section("OUTPUT", &message.message, failed, cx));

    let element = disclosure(
        &id,
        Head {
            icon: status_icon(status, failed, loading),
            tint: status_tint(status, failed, cx),
            name: message
                .tool_name
                .clone()
                .unwrap_or_else(|| "tool".to_string()),
            summary: message.tool_kind.clone(),
        },
        open,
        body.into_iter().collect(),
        cx,
    );
    (id, element)
}

/// An Ollama tool call and the result it produced.
pub(super) fn pair(
    message: &AgentMessage,
    block: &ToolBlock,
    result: Option<&AgentMessage>,
    open: bool,
    cx: &App,
) -> (SharedString, AnyElement) {
    let call = message
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.get(block.call));
    let id = SharedString::from(call.map(|call| call.id.clone()).unwrap_or_default());
    let name = call.map_or_else(|| "tool".to_string(), |call| call.name.clone());

    let mut body = Vec::new();
    if let Some(call) = call {
        body.extend(arguments(&call.args, cx));
    }
    if let Some(result) = result.filter(|result| !result.message.trim().is_empty()) {
        body.push(section("RESULT", &result.message, block.is_error, cx));
    }

    let element = disclosure(
        &id,
        Head {
            icon: if block.is_error {
                IconName::TriangleAlert
            } else {
                IconName::Wrench
            },
            tint: if block.is_error {
                cx.peek_theme().red
            } else {
                cx.peek_theme().fg_subtle
            },
            name,
            summary: call.and_then(|call| summary(&call.args)),
        },
        open,
        body,
        cx,
    );
    (id, element)
}

struct Head {
    icon: IconName,
    tint: Hsla,
    name: String,
    summary: Option<String>,
}

fn disclosure(
    id: &SharedString,
    head: Head,
    open: bool,
    body: Vec<AnyElement>,
    cx: &App,
) -> AnyElement {
    let theme = cx.peek_theme();
    let trigger = Button::new(SharedString::from(format!("tool-{id}")))
        .ghost()
        .w_full()
        .justify_start()
        .child(
            div()
                .h_flex()
                .w_full()
                .items_center()
                .gap(rems(0.4))
                .child(
                    Icon::new(head.icon)
                        .size(rems(0.8125))
                        .text_color(head.tint),
                )
                .child(
                    div()
                        .text_size(rems(0.72))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(theme.fg)
                        .child(head.name),
                )
                .children(head.summary.map(|summary| {
                    div()
                        .flex_1()
                        .truncate()
                        .text_size(rems(0.65))
                        .text_color(theme.fg_subtle)
                        .child(format!("· {summary}"))
                }))
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(rems(0.875))
                    .text_color(theme.fg_subtle),
                ),
        );

    Collapsible::new()
        .open(open)
        .child(trigger)
        .content(
            div()
                .v_flex()
                .gap(rems(0.5))
                .ml(rems(0.5))
                .pl(rems(0.9))
                .border_l_1()
                .border_color(theme.node_border)
                .children(body),
        )
        .into_any_element()
}

/// The call's arguments, with SQL split out: a query is the one argument a reader actually
/// wants to read, and burying it inside pretty-printed JSON hides it.
fn arguments(args: &serde_json::Value, cx: &App) -> Vec<AnyElement> {
    let mut blocks = Vec::new();
    let Some(object) = args.as_object() else {
        return blocks;
    };
    if let Some(sql) = object.get("query").and_then(serde_json::Value::as_str) {
        blocks.push(section("SQL", sql, false, cx));
    }
    let rest: serde_json::Map<_, _> = object
        .iter()
        .filter(|(key, _)| key.as_str() != "query")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !rest.is_empty() {
        let text = serde_json::to_string_pretty(&rest).unwrap_or_default();
        blocks.push(section("ARGUMENTS", &text, false, cx));
    }
    blocks
}

/// The one-line gist shown beside a collapsed call, from whichever argument names it best.
fn summary(args: &serde_json::Value) -> Option<String> {
    for key in ["title", "name", "nodeId", "node_id", "pageId"] {
        if let Some(value) = args.get(key).and_then(serde_json::Value::as_str) {
            return Some(value.to_string());
        }
    }
    None
}

fn section(label: &str, body: &str, is_error: bool, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    div()
        .v_flex()
        .gap(rems(0.3))
        .child(
            div()
                .text_size(rems(0.58))
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_color(theme.fg_subtle)
                .child(label.to_string()),
        )
        .child(
            div()
                .p(rems(0.5))
                .rounded(scaled(theme.radius_card))
                .bg(theme.node_inset)
                .border_1()
                .border_color(theme.node_border)
                .text_size(rems(0.7))
                .text_color(if is_error { theme.red } else { theme.fg })
                .child(body.to_string()),
        )
        .into_any_element()
}

/// A persisted `pending` row from a session that died must not spin forever, so the spinner is
/// gated on the turn actually being live.
fn status_icon(status: &str, failed: bool, loading: bool) -> IconName {
    if failed {
        return IconName::TriangleAlert;
    }
    match status {
        "completed" => IconName::CircleCheck,
        "pending" | "in_progress" if loading => IconName::LoaderCircle,
        _ => IconName::Wrench,
    }
}

fn status_tint(status: &str, failed: bool, cx: &App) -> Hsla {
    let theme = cx.peek_theme();
    if failed {
        return theme.red;
    }
    match status {
        "completed" => theme.green,
        _ => theme.fg_subtle,
    }
}
