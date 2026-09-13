//! The pane that shows one cell's full value.
//!
//! The reference has nothing like it: a JSON cell there renders its whole tree inside the cell
//! and the row grows to fit. `DataTable` virtualises with `uniform_list`, which needs every row
//! the same height, so the grid shows a one-line summary and the full value opens here.
//!
//! **Inside the node, not floating over it.** An overlay was the first plan, and it is the wrong
//! shape for this: gpui lays overlays out at rem 1 whatever the camera is doing, so a popover
//! would not scale with the node it belongs to, and `deferred` inherits the content mask in
//! force where it was created — which for a node body is `overflow_hidden`, so it would be
//! clipped by the very table it is explaining. The Variable node reached the same conclusion and
//! expands its list editor inline for the same reason (`docs/canvas.md`, "Editable nodes").

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, SharedString, div, rems};
use peek_document::Cell;
use peek_theme::ActivePeekTheme;

use super::ResultTable;
use super::json::{self, Token};

/// Longer than this and a value opens in the pane rather than an in-cell field, which would
/// clip it. Matches the middle-truncation threshold, so anything the grid shortens is editable
/// somewhere it fits whole.
pub(super) const INLINE_LIMIT: usize = 36;

/// How tall the pane is allowed to get before it scrolls, in rems of the node's own scale.
const MAX_HEIGHT: f32 = 11.0;

impl ResultTable {
    /// The detail pane, or nothing when no cell is open.
    pub(super) fn detail_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let table = self.table.read(cx);
        let delegate = table.delegate();
        let (row, column) = delegate.detail()?;
        let value = delegate.result_rows().cell(row, column)?.clone();
        let name = delegate
            .result_rows()
            .columns()
            .get(column)
            .map_or_else(String::new, |column| column.name.clone());

        let theme = cx.peek_theme();
        Some(
            div()
                .v_flex()
                .flex_none()
                .w_full()
                .max_h(rems(MAX_HEIGHT))
                .border_b_1()
                .border_color(theme.node_border)
                .bg(theme.node_inset)
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .flex_none()
                        .px(rems(0.5))
                        .py(rems(0.25))
                        .text_size(rems(0.625))
                        .text_color(theme.fg_subtle)
                        .child(name)
                        .child(
                            Button::new(SharedString::from(format!("{}-detail-close", self.node)))
                                .ghost()
                                .xsmall()
                                .label("Close")
                                .tooltip("Close the value pane")
                                .on_click(cx.listener(|this, _, _, cx| this.close_detail(cx))),
                        ),
                )
                .child(body(&value, cx))
                .into_any_element(),
        )
    }
}

/// A JSON value as a tree; anything else as its own text, wrapped.
fn body(value: &Cell, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    let pane = div()
        .id("result-detail-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(rems(0.625))
        .pb(rems(0.5))
        .font_family("Monaspace Krypton")
        .text_size(rems(0.6875));

    match value {
        Cell::Json(json) => pane
            .v_flex()
            .children(
                json::lines(json)
                    .into_iter()
                    .enumerate()
                    .map(|(index, line)| tree_line(line, index, cx)),
            )
            .into_any_element(),
        Cell::Null => pane
            .child(div().italic().text_color(theme.fg_muted).child("NULL"))
            .into_any_element(),
        Cell::Undecodable => pane
            .child(
                div()
                    .italic()
                    .text_color(theme.red)
                    .child("The driver could not decode this value"),
            )
            .into_any_element(),
        other => pane
            .child(
                div()
                    .w_full()
                    .text_color(theme.fg)
                    .child(other.to_display_string()),
            )
            .into_any_element(),
    }
}

/// One line of the tree: its indent, an optional key, the value, and a char count when the value
/// was middle-truncated.
fn tree_line(line: json::Line, index: usize, cx: &App) -> impl IntoElement {
    let theme = cx.peek_theme();
    let color = match line.token {
        Token::Brace => theme.fg_subtle,
        Token::Null | Token::Empty => theme.fg_muted,
        Token::Bool => theme.blue,
        Token::Number => theme.yellow,
        Token::Text => theme.green,
    };

    #[allow(
        clippy::cast_precision_loss,
        reason = "a JSON nesting depth is a handful"
    )]
    let indent = rems(line.depth as f32 * 0.75);

    div()
        .id(("json-line", index))
        .h_flex()
        .items_start()
        .gap(rems(0.25))
        .pl(indent)
        .children(
            line.key
                .map(|key| div().flex_none().text_color(theme.fg_muted).child(key)),
        )
        .child(div().min_w_0().text_color(color).child(format!(
            "{}{}",
            line.text,
            if line.comma { "," } else { "" }
        )))
        .children(line.char_count.map(|count| {
            div()
                .flex_none()
                .px(rems(0.1875))
                .rounded(theme.radius_pill)
                .bg(theme.node_bg_2)
                .text_size(rems(0.5))
                .text_color(theme.fg_subtle)
                .child(format!("{count}ch"))
        }))
}
