//! What one cell looks like.
//!
//! Ported from `~/labs/peek/src/canvas/nodes/Result/cell/Cell.tsx`, which dispatches in a fixed
//! order — JSON, then text and numbers, then booleans, then NULL. The order has consequences
//! worth keeping: a boolean that arrives as the *string* `"true"` renders as text rather than as
//! the TRUE pill, and a NULL in a JSON column falls through to the NULL span.
//!
//! One deliberate difference: the reference renders a JSON cell's whole pretty-printed tree
//! inline and lets the row grow to fit. `DataTable` virtualises with `uniform_list`, which
//! requires every row to be the same height, so a JSON cell shows a one-line summary here and
//! opens in a detail panel instead.

use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Hsla, div, px, rems};
use peek_document::Cell;

use super::column_roles::Role;
use peek_theme::{ActivePeekTheme, PeekTheme};

/// The colour a key column's values take: `--pk-yellow` for a primary key, `--pk-blue` for a
/// foreign key, so a reference is recognisable at a glance without reading the header.
fn role_color(role: Role, theme: &PeekTheme) -> Option<Hsla> {
    match role {
        Role::PrimaryKey => Some(theme.yellow),
        Role::ForeignKey => Some(theme.blue),
        Role::Plain => None,
    }
}

/// The alphas `.reference` mixes its chip out of its `--chip-color`: a barely-there fill under a
/// border you can actually see.
const CHIP_FILL: f32 = 0.11;
const CHIP_BORDER: f32 = 0.28;
/// `.reference`'s own corner, which is smaller than anything in the theme's radius scale because
/// the chip has to sit inside a table row without touching its neighbours.
const CHIP_RADIUS: f32 = 5.0;
/// The `PK` / `FK` tag next to a column name (`node.css`, `.col-tag`).
const TAG_RADIUS: f32 = 3.0;

/// Names the header cell so hovering it can tint the column name inside it, which is
/// `thead th:hover .col-name` — the one cue that a header is a control.
pub(super) const HEADER_GROUP: &str = "result-th";

/// A value's one-line rendering.
pub(super) fn cell(value: &Cell, role: Role, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    if let Some(color) = role_color(role, theme) {
        // A key or a reference is tinted whatever its type, because what matters about it is
        // that it identifies a row rather than what it holds.
        if !matches!(value, Cell::Null | Cell::Undecodable | Cell::Json(_)) {
            return chip(&value.to_display_string(), color);
        }
    }
    match value {
        Cell::Json(_) => summary(value, theme),
        Cell::Bool(flag) => boolean(*flag, theme),
        Cell::Null => marker("NULL", theme.fg_muted),
        // A decode failure is not a NULL, and saying so beats an empty cell the user would read
        // as "this row has no value here".
        Cell::Undecodable => marker("unreadable", theme.red),
        // `Result.css` has no alignment rule for a `td`, so a number reads left like everything
        // else. The table is already mono, so its digits line up regardless.
        Cell::Int(_) | Cell::Float(_) => plain(&value.to_display_string(), theme),
        Cell::Text(text) => plain(text, theme),
    }
}

fn plain(text: &str, theme: &PeekTheme) -> AnyElement {
    div()
        .truncate()
        .text_color(theme.fg)
        .child(text.to_string())
        .into_any_element()
}

/// `.reference`: a key or a reference value reads as a token rather than as text, so a row's
/// identity is findable without reading it. The fill and border are mixed out of the role's own
/// colour, which is what keeps one rule working for both the yellow and the blue.
fn chip(text: &str, color: Hsla) -> AnyElement {
    div()
        .h_flex()
        .items_center()
        .flex_none()
        .max_w_full()
        .px(rems(0.4375))
        .rounded(px(CHIP_RADIUS))
        .border_1()
        .border_color(color.opacity(CHIP_BORDER))
        .bg(color.opacity(CHIP_FILL))
        .text_color(color)
        .child(div().truncate().child(text.to_string()))
        .into_any_element()
}

/// `TRUE`/`FALSE` in the two semantic colours the stylesheet gives them.
fn boolean(flag: bool, theme: &PeekTheme) -> AnyElement {
    let (label, color) = if flag {
        ("TRUE", theme.blue)
    } else {
        ("FALSE", theme.red)
    };
    marker(label, color)
}

/// An italic, muted word standing in for a value rather than being one.
fn marker(label: &'static str, color: Hsla) -> AnyElement {
    div()
        .truncate()
        .italic()
        .text_color(color)
        .child(label)
        .into_any_element()
}

/// `{…} 3 keys` / `[…] 7 items`, so the shape and size of a JSON value read at a glance without
/// the row having to grow to hold it. A scalar in a JSON column shows as itself.
fn summary(value: &Cell, theme: &PeekTheme) -> AnyElement {
    let text = value
        .json_summary()
        .unwrap_or_else(|| value.to_display_string());
    div()
        .truncate()
        .text_color(theme.fg_muted)
        .child(text)
        .into_any_element()
}

/// A header cell: the column's name over its type, with a `PK`/`FK` tag when the column is one.
pub(super) fn header(name: &str, sql_type: &str, role: Role, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    let tag = match role {
        Role::PrimaryKey => Some(("PK", theme.yellow, theme.yellow_soft)),
        Role::ForeignKey => Some(("FK", theme.blue, theme.blue_soft)),
        Role::Plain => None,
    };
    div()
        .v_flex()
        .justify_center()
        .size_full()
        .gap(rems(0.0625))
        .child(
            div()
                .h_flex()
                .items_center()
                .gap(rems(0.25))
                .min_w_0()
                // Sizes to its content, so the type line below keeps its room rather than being
                // pushed out of the header's clipped box.
                .flex_none()
                .child(
                    div()
                        .truncate()
                        .text_color(tag.map_or(theme.fg, |(_, color, _)| color))
                        .group_hover(HEADER_GROUP, |name| name.text_color(theme.accent_soft))
                        .child(name.to_string()),
                )
                .children(tag.map(|(label, color, soft)| {
                    div()
                        .flex_none()
                        .px(rems(0.25))
                        .rounded(px(TAG_RADIUS))
                        .text_size(rems(0.5625))
                        .text_color(color)
                        .bg(soft)
                        .child(label)
                })),
        )
        .child(
            div()
                .truncate()
                .flex_none()
                .text_size(rems(0.59375))
                .text_color(theme.fg_subtle)
                // `.col-type { text-transform: uppercase }`: the type is a label about the
                // column, not a value, and the case is what keeps it from reading as one.
                .child(sql_type.to_uppercase()),
        )
        .into_any_element()
}
