//! The `TableDefinition` node: `~/labs/peek/src/canvas/nodes/TableDefinition/`.
//!
//! Read-only: a two-column table of column name and type, the type coloured by the category
//! `column_type` sorts it into. PK and FK badges are missing because their source is missing:
//! `TableDefinitionTable.tsx` reads `schemaAtom` (`primaryKeys`, `references`), which peek-db
//! introduces in M5. The badges land with it, not before — the name-shape heuristics beside
//! them there are a tie-breaker for the schema, not a substitute for it.
//!
//! The body does not scroll, and a longer table is clipped as `.app-node-body { overflow:
//! hidden }` clips it. The canvas' wheel listener is gated on `should_handle_scroll`, which its
//! `HitboxBehavior::Normal` hitbox permits, and it registers last — so gpui, bubbling in reverse
//! registration order, hands it every wheel before any node descendant sees one. A scrollable
//! body would therefore scroll *and* pan. In-node scrolling needs wheel arbitration in
//! `canvas/mod.rs`, once, for every kind.

mod column_type;

use gpui_kit::TestSupportExt;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Hsla, SharedString, div, rems};
use peek_document::TableDefinitionData;
use peek_theme::{ActivePeekTheme, PeekTheme};

use column_type::TypeCategory;

pub(crate) fn title(data: &TableDefinitionData) -> String {
    if data.table.is_empty() {
        "untitled".to_string()
    } else {
        data.table.clone()
    }
}

pub(crate) fn body(data: &TableDefinitionData, cx: &mut App) -> AnyElement {
    let theme = cx.peek_theme();
    let last = data.columns.len().saturating_sub(1);
    div()
        .v_flex()
        .size_full()
        .text_size(rems(0.75))
        .children(
            data.columns
                .iter()
                .enumerate()
                .map(|(index, column)| row(column, theme, index == last)),
        )
        .into_any_element()
}

/// One `name | type` row, identified by its column name. The last one drops the rule so the
/// table ends on the body edge.
fn row(column: &(String, String), theme: &PeekTheme, is_last: bool) -> impl IntoElement {
    let (name, column_type) = column;
    let hover_bg = theme.node_bg_2;
    div()
        .h_flex()
        .gap(rems(0.75))
        .px(rems(0.875))
        .py(rems(0.375))
        .flex_shrink_0()
        .when(!is_last, |row| {
            row.border_b_1().border_color(theme.node_border)
        })
        .hover(move |style| style.bg(hover_bg))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(theme.fg)
                .child(name.clone()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(rems(0.625))
                .text_color(type_color(TypeCategory::of(column_type), theme))
                .child(column_type.to_uppercase()),
        )
        .id(SharedString::from(name.clone()))
        // The visible type is upper-cased for the eye; the label keeps the real spelling.
        .aria_label(SharedString::from(format!("{name} {column_type}")))
        .test_support()
}

/// `TableDefinition.css`'s `.col-type.type-*` rules, one role each. The json and uuid colours
/// the CSS hardcoded are `magenta` and `cyan`, which the dark themes match exactly and the two
/// light ones darken to stay legible on their node background.
fn type_color(category: TypeCategory, theme: &PeekTheme) -> Hsla {
    match category {
        TypeCategory::Numeric => theme.blue,
        TypeCategory::Text => theme.green,
        TypeCategory::Boolean => theme.accent_soft,
        TypeCategory::Datetime => theme.yellow,
        TypeCategory::Json => theme.magenta,
        TypeCategory::Uuid => theme.cyan,
        TypeCategory::Binary | TypeCategory::Other => theme.fg_muted,
    }
}
