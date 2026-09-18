//! The `QueryError` node: `~/labs/peek/src/canvas/nodes/QueryError/QueryErrorNode.tsx`.
//!
//! The body is that file's `.error-shape`: a `red_soft` panel behind a `red` rule, carrying the
//! database's own words. The header is always `query failed`; the shell draws the indicator.
//!
//! `Suggest fix` and `Accept` are absent, and one is the reason for the other. `Suggest fix`
//! streams an LLM completion whose system prompt embeds the live schema (`schemaAtom`) and the
//! connection's dialect (`activeEngineAtom`) through `useExecutePrompt`; none of those exist
//! yet. `Accept` writes *that stream's* result into the originating query node and nothing
//! else — the reference renders it only while the suggestion is non-empty — so shipping it
//! alone would be a button that writes an empty query over a real one. Both arrive together
//! with the agent backend in M6, along with the node's self-resize to suggestion height + 250.
//! Whoever adds those buttons must keep them out of the outer resize band and the header band:
//! `peek_canvas::hit::node_hit_at` claims those bands for resize and header drag in world
//! space, so a control there never sees the press.
//!
//! The body does not scroll: the canvas' window-level wheel listener is gated on
//! `should_handle_scroll`, which an ordinary node hitbox permits, so the wheel pans the canvas
//! rather than reaching anything inside a node. A long error is clipped until the shell owns
//! scrolling for every kind — the reference clips it too (`.app-node-body { overflow: hidden }`)
//! and leaves the node resizable.

use gpui_kit::TestSupportExt;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, SharedString, Window, div, rems};
use peek_document::{ErrorData, NodeId};
use peek_theme::ActivePeekTheme;

use super::kind::NodeContext;

/// `border-top: 2px solid var(--pk-red)` in `QueryError.css`, drawn as a sliver rather than a
/// border so it scales with the camera's rem scope; the shell's borders are physical hairlines.
const RULE_HEIGHT: f32 = 0.125;

pub(crate) fn title(_data: &ErrorData) -> String {
    "query failed".to_string()
}

pub(crate) fn body(
    id: &NodeId,
    data: &ErrorData,
    _context: NodeContext<'_>,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.peek_theme();
    let message = SharedString::from(data.message.clone());
    div()
        .v_flex()
        .size_full()
        .bg(theme.red_soft)
        .child(div().h(rems(RULE_HEIGHT)).flex_shrink_0().bg(theme.red))
        .child(
            div()
                .id(SharedString::from(format!("{id}-message")))
                .aria_label(message.clone())
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .p(rems(1.0))
                .text_size(rems(0.78125))
                .line_height(rems(1.171_875))
                .text_color(theme.fg)
                .child(message),
        )
        .into_any_element()
}
