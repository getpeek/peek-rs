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
use gpui_kit::{AnyElement, App, Hsla, div, rems};
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

/// A value's one-line rendering.
pub(super) fn cell(value: &Cell, sql_type: &str, role: Role, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    if let Some(color) = role_color(role, theme) {
        // A key or a reference is tinted whatever its type, because what matters about it is
        // that it identifies a row rather than what it holds.
        if !matches!(value, Cell::Null | Cell::Undecodable | Cell::Json(_)) {
            return div()
                .truncate()
                .text_color(color)
                .child(value.to_display_string())
                .into_any_element();
        }
    }
    match value {
        Cell::Json(_) => summary(value, theme),
        Cell::Bool(flag) => boolean(*flag, theme),
        Cell::Null => marker("NULL", theme.fg_muted),
        // A decode failure is not a NULL, and saying so beats an empty cell the user would read
        // as "this row has no value here".
        Cell::Undecodable => marker("unreadable", theme.red),
        Cell::Int(_) | Cell::Float(_) => numeric(&value.to_display_string(), theme),
        Cell::Text(text) if peek_document::is_numeric(sql_type) => numeric(text, theme),
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

/// Numbers are right-aligned with tabular figures so digits line up column-wise, which is what
/// makes a column of amounts comparable at a glance.
fn numeric(text: &str, theme: &PeekTheme) -> AnyElement {
    div()
        .w_full()
        .truncate()
        .text_right()
        .font_family("Monaspace Krypton")
        .text_color(theme.fg)
        .child(text.to_string())
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
        Role::PrimaryKey => Some(("PK", theme.yellow)),
        Role::ForeignKey => Some(("FK", theme.blue)),
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
                        .text_color(tag.map_or(theme.fg, |(_, color)| color))
                        .child(name.to_string()),
                )
                .children(tag.map(|(label, color)| {
                    div()
                        .flex_none()
                        .px(rems(0.1875))
                        .rounded(theme.radius_pill)
                        .text_size(rems(0.5))
                        .text_color(color)
                        .bg(theme.node_bg_2)
                        .child(label)
                })),
        )
        .child(
            div()
                .truncate()
                .flex_none()
                .text_size(rems(0.5625))
                .text_color(theme.fg_subtle)
                .child(sql_type.to_lowercase()),
        )
        .into_any_element()
}
