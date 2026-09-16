//! The pane that shows one cell's full value.
//!
//! The reference has nothing like it: a JSON cell there renders its whole tree inside the cell
//! and the row grows to fit. `DataTable` virtualises with `uniform_list`, which needs every row
//! the same height, so the grid shows a one-line preview and the full value opens here — as a
//! foldable, searchable tree.
//!
//! **Inside the node, not floating over it.** An overlay was the first plan, and it is the wrong
//! shape for this — though not for the reasons first written here, which were wrong both ways
//! round. A `deferred` draw raised from inside a node body **does** scale with the camera:
//! `defer_draw` captures the rem size in force and both deferred passes re-enter
//! `with_rem_size`, which `node/query/mod.rs` found the hard way. And it is **not** clipped by
//! the body's `overflow_hidden`: `deferred()` passes `content_mask: None`, and
//! `with_content_mask(None)` is a no-op. What actually decides it is that a pane explaining a
//! cell is *content* — it belongs to the node and should scale with it — and content that
//! pushes the table down is easier to read than content floating over it. The Variable node
//! expands its list editor inline for the same reason (`docs/canvas.md`, "Editable nodes"); the
//! right-click menu is chrome and goes the other way, onto the canvas. So does the JSON editor
//! (`canvas/json_editor.rs`): a field you type into should not shrink with the camera.
//!
//! **The tree is parsed once per opened cell, not once per frame.** The pane shows exactly one
//! cell, so there is no cache to key and no eviction to get wrong: [`Detail`] holds the parsed
//! view beside the coordinates it was parsed for, and is dropped when either the cell moves or
//! the rows are replaced. Rendering only reads it — `json::View::line` borrows.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Icon, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, Window, div, px, rems, uniform_list};
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
/// One tree line, in rems. Uniform, because `uniform_list` measures the first and assumes the
/// rest — the same constraint the grid's rows are under.
const LINE_HEIGHT: f32 = 1.0625;
/// How far one nesting level indents.
const INDENT: f32 = 0.75;

/// The open cell and the tree parsed from it.
///
/// `view` is `None` for anything that is not JSON — a long string has no structure to fold, and
/// parsing one would only produce a single line saying so.
pub(super) struct Detail {
    pub(super) row: usize,
    pub(super) column: usize,
    view: Option<json::View>,
}

impl std::fmt::Debug for Detail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Detail")
            .field("row", &self.row)
            .field("column", &self.column)
            .finish_non_exhaustive()
    }
}

impl ResultTable {
    /// The detail pane, or nothing when no cell is open.
    pub(super) fn detail_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (row, column) = self.table.read(cx).delegate().detail()?;
        self.sync_detail((row, column), window, cx);
        let value = self
            .table
            .read(cx)
            .delegate()
            .result_rows()
            .cell(row, column)?
            .clone();
        let name = self
            .table
            .read(cx)
            .delegate()
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
                .child(self.detail_header(name, cx))
                .child(self.detail_body(&value, cx))
                .into_any_element(),
        )
    }

    /// Parses the open cell when it is not the one already parsed.
    ///
    /// Called from the render path, so the common case has to be a comparison and nothing else —
    /// which it is: two `usize`s against the cached pair.
    fn sync_detail(&mut self, cell: (usize, usize), window: &mut Window, cx: &mut Context<Self>) {
        let (row, column) = cell;
        if self
            .detail
            .as_ref()
            .is_some_and(|open| open.row == row && open.column == column)
        {
            return;
        }
        let view = match self
            .table
            .read(cx)
            .delegate()
            .result_rows()
            .cell(row, column)
        {
            Some(Cell::Json(value)) => Some(json::View::open(value)),
            _ => None,
        };
        self.detail = Some(Detail { row, column, view });
        // A query typed against the previous value means nothing against this one.
        self.detail_search
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Drops the parsed tree. Called when the rows are replaced: the coordinates it holds mean
    /// nothing against a different result.
    pub(super) fn forget_detail(&mut self) {
        self.detail = None;
    }

    /// The column name, the fold controls, and a search field — but only the controls that have
    /// something to act on, so a long string does not offer to collapse a structure it has not
    /// got.
    fn detail_header(&self, name: String, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme();
        let foldable = self
            .detail
            .as_ref()
            .and_then(|open| open.view.as_ref())
            .is_some_and(json::View::foldable);
        let found = self
            .detail
            .as_ref()
            .and_then(|open| open.view.as_ref())
            .filter(|view| view.is_searching())
            .map(json::View::matches);

        div()
            .h_flex()
            .items_center()
            .gap(rems(0.375))
            .flex_none()
            .px(rems(0.5))
            .py(rems(0.25))
            .text_size(rems(0.625))
            .text_color(theme.fg_subtle)
            .child(div().flex_none().child(name))
            .child(div().flex_1())
            .children(found.map(|count| {
                div()
                    .flex_none()
                    .child(if count == 1 {
                        "1 match".to_string()
                    } else {
                        format!("{count} matches")
                    })
                    .into_any_element()
            }))
            .when(foldable, |header| {
                header
                    .child(
                        div()
                            .w(rems(7.0))
                            .child(Input::new(&self.detail_search).xsmall()),
                    )
                    .child(
                        Button::new(self.ids.detail_collapse.clone())
                            .ghost()
                            .xsmall()
                            .label("Collapse all")
                            .on_click(cx.listener(|this, _, _, cx| this.fold_detail(true, cx))),
                    )
                    .child(
                        Button::new(self.ids.detail_expand.clone())
                            .ghost()
                            .xsmall()
                            .label("Expand all")
                            .on_click(cx.listener(|this, _, _, cx| this.fold_detail(false, cx))),
                    )
            })
            .child(
                Button::new(self.ids.detail_close.clone())
                    .ghost()
                    .xsmall()
                    .label("Close")
                    .tooltip("Close the value pane")
                    .on_click(cx.listener(|this, _, _, cx| this.close_detail(cx))),
            )
            .into_any_element()
    }

    /// A JSON value as a tree; anything else as its own text, wrapped.
    fn detail_body(&self, value: &Cell, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme();
        let pane = div()
            .id("result-detail-body")
            .flex_1()
            .min_h_0()
            .px(rems(0.625))
            .pb(rems(0.5))
            .font_family("Monaspace Krypton")
            .text_size(rems(0.6875));

        if self.detail.as_ref().is_some_and(|open| open.view.is_some()) {
            return pane.child(self.tree(cx)).into_any_element();
        }

        match value {
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

    /// The tree, virtualised.
    ///
    /// A JSON document with tens of thousands of nodes is ordinary in a jsonb column, and the
    /// pane used to build an element for every line of it. `uniform_list` builds the dozen that
    /// are on screen, which is what `DataTable` already does for the grid's rows.
    fn tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(view) = self.detail.as_ref().and_then(|open| open.view.as_ref()) else {
            return div().into_any_element();
        };
        let entity = cx.entity();
        uniform_list(
            "result-detail-tree",
            view.visible(),
            move |range, _window, cx| {
                let table = entity.read(cx);
                let Some(view) = table.detail.as_ref().and_then(|open| open.view.as_ref()) else {
                    return Vec::new();
                };
                range
                    .filter_map(|position| {
                        let line = view.line(position)?;
                        Some(tree_line(&line, position, &entity, cx))
                    })
                    .collect()
            },
        )
        .h(rems(LINE_HEIGHT * lines_shown(view.visible())))
        .into_any_element()
    }

    fn fold_detail(&mut self, folded: bool, cx: &mut Context<Self>) {
        if let Some(view) = self.detail.as_mut().and_then(|open| open.view.as_mut()) {
            view.set_all_folded(folded);
            cx.notify();
        }
    }

    fn toggle_fold(&mut self, arena: usize, cx: &mut Context<Self>) {
        if let Some(view) = self.detail.as_mut().and_then(|open| open.view.as_mut()) {
            view.toggle(arena);
            cx.notify();
        }
    }

    /// Searches the open value.
    ///
    /// Not debounced, unlike the grid's find: this scans one parsed value rather than every cell
    /// of a result, and the arena is already in memory. A timer here would only add latency.
    pub(super) fn on_detail_search_changed(&mut self, query: &str, cx: &mut Context<Self>) {
        if let Some(view) = self.detail.as_mut().and_then(|open| open.view.as_mut()) {
            view.search(query);
            cx.notify();
        }
    }
}

/// How tall the list is: its content, capped so a short value does not leave the pane padded out
/// with empty rows.
fn lines_shown(lines: usize) -> f32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a line count far beyond f32's exact range is far beyond what is drawn"
    )]
    let lines = lines as f32;
    lines.min(MAX_HEIGHT / LINE_HEIGHT)
}

/// One line of the tree: its fold control, indent, key, value, and a char count when the value
/// was middle-truncated.
fn tree_line(
    line: &json::Line<'_>,
    position: usize,
    table: &Entity<ResultTable>,
    cx: &App,
) -> AnyElement {
    let theme = cx.peek_theme();
    let color = match line.token {
        Token::Brace => theme.fg_subtle,
        Token::Null | Token::Empty => theme.fg_muted,
        Token::Bool => theme.blue,
        Token::Number => theme.yellow,
        Token::Text => theme.green,
    };
    // A search dims what did not match rather than hiding it: a key means little without the
    // structure around it, and a tree with holes in it is harder to read than a dim one.
    let dimmed = line.fold.is_none() && !line.matched && searching(table, cx);
    let opacity = if dimmed { 0.35 } else { 1.0 };

    #[allow(
        clippy::cast_precision_loss,
        reason = "a JSON nesting depth is a handful"
    )]
    let indent = rems(line.depth as f32 * INDENT);

    div()
        .id(("json-line", position))
        .h_flex()
        .items_center()
        .gap(rems(0.25))
        .h(rems(LINE_HEIGHT))
        .pl(indent)
        .when(line.matched, |row| row.bg(theme.node_bg_2))
        .child(fold_control(line, position, table, cx))
        .children(line.key.map(|key| {
            div()
                .flex_none()
                .text_color(theme.fg_muted.opacity(opacity))
                .child(key.to_string())
        }))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(color.opacity(opacity))
                .child(format!(
                    "{}{}",
                    line.text,
                    if line.comma { "," } else { "" }
                )),
        )
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
        .into_any_element()
}

/// The chevron that folds a container, or a gap the width of one so every line's text starts at
/// the same place whether or not it can be folded.
fn fold_control(
    line: &json::Line<'_>,
    position: usize,
    table: &Entity<ResultTable>,
    cx: &App,
) -> AnyElement {
    let gutter = rems(0.75);
    let Some(folded) = line.fold else {
        return div().flex_none().w(gutter).into_any_element();
    };
    let icon = if folded {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    let arena = line.arena;
    let table = table.downgrade();
    div()
        .id(("json-fold", position))
        .flex_none()
        .w(gutter)
        .text_color(cx.peek_theme().fg_subtle)
        .cursor_pointer()
        .child(Icon::new(icon).size(px(9.0)))
        .on_click(move |_, _, cx| {
            table
                .update(cx, |table, cx| table.toggle_fold(arena, cx))
                .ok();
        })
        .into_any_element()
}

fn searching(table: &Entity<ResultTable>, cx: &App) -> bool {
    table
        .read(cx)
        .detail
        .as_ref()
        .and_then(|open| open.view.as_ref())
        .is_some_and(json::View::is_searching)
}

/// The pane's search field, a sibling of the grid's so the two look alike.
pub(super) fn search_input(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder("Find in value"))
}
