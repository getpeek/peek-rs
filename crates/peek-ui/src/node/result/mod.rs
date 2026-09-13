//! The Result node: `~/labs/peek/src/canvas/nodes/Result/`.
//!
//! A virtualised table over the rows a query returned. The rows live on the session
//! [`Document`] (and in the `<connection>.results.json` sidecar), not in the node's data, so a
//! 6,515-row result costs the document nothing.
//!
//! Two deliberate differences from the reference, both forced by `DataTable` virtualising with
//! `uniform_list`, which measures the first row and assumes the rest match:
//!
//! - **Rows are a uniform height.** The reference measures each row, because a JSON cell renders
//!   its whole pretty-printed tree inline.
//! - **JSON cells show a one-line summary.** They open in a detail panel instead of growing the
//!   row. That is what every other database GUI does.
//!
//! Not yet here, and each waits on something named: cell and row selection, the toolbar, search,
//! the JSON detail panel, inline editing and the context menus. Selection in particular waits on
//! pointer arbitration — the canvas's window-level listeners run before any node element, so a
//! press inside the table is currently the canvas's, not the table's.

mod aggregate;
mod cells;
mod column_roles;
pub(crate) mod delegate;
mod delete;
mod detail;
mod edit;
mod editable;
mod follow;
mod json;
pub(crate) mod pivot;
mod search;
mod selection;
mod toolbar;
mod widths;

use std::sync::Arc;

use gpui_kit::TestSupportExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{Sizable, Size, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, Render, SharedString, Task, Window, div, px, rems,
};
use peek_canvas::Document;
use std::collections::BTreeMap;

use peek_document::{NodeId, ResultData, ResultSet};
use peek_theme::ActivePeekTheme;

use delegate::ROW_HEIGHT;
pub(crate) use delegate::ResultDelegate;

use super::kind::NodeContext;
use super::state::NodeState;

/// `useResultSearchMatches`'s debounce before a query is matched against every cell.
const SEARCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(100);

/// The header's own title, `nodeHeading` in `queryHeading.ts`: the query's first meaningful line,
/// cut at 60 characters.
const HEADING_LIMIT: usize = 60;

pub(crate) fn title(data: &ResultData) -> String {
    heading(&data.query)
}

/// The same cut applied to raw SQL, for callers holding a query rather than a node's data —
/// page search names a result by the statement behind it.
pub(crate) fn heading(query: &str) -> String {
    let line = query
        .lines()
        .map(|line| line.trim_start_matches("--").trim())
        .find(|line| !line.is_empty())
        .unwrap_or("result");
    if line.chars().count() <= HEADING_LIMIT {
        return line.to_string();
    }
    let cut: String = line.chars().take(HEADING_LIMIT).collect();
    format!("{cut}...")
}

/// Retained state: the table, which owns the scroll position and the visible range, wrapped in
/// an entity so it has somewhere to keep the subscription that persists a column resize.
pub(crate) struct ResultState {
    inner: Entity<ResultTable>,
}

impl std::fmt::Debug for ResultState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResultState")
            .finish_non_exhaustive()
    }
}

/// Owns the table, the toolbar above it, and the find bar that replaces the toolbar.
///
/// An entity rather than a plain render function because all three keep state between frames:
/// the table its scroll position, the find bar its text and its debounce.
pub(crate) struct ResultTable {
    pub(super) table: Entity<TableState<ResultDelegate>>,
    pub(super) node: NodeId,
    document: Entity<Document>,
    /// Tables the query reads, for the toolbar's badges. Recomputed only when the SQL changes:
    /// parsing a statement every frame would be wasteful and the answer never moves on its own.
    pub(super) tables: Vec<SharedString>,
    /// The single table this result can be written through, from the same parse as `tables`.
    /// The toolbar asks on every frame — a delete button only exists for a writable result —
    /// and answering it with a fresh tree-sitter parse per frame was exactly that waste.
    pub(super) editable: Option<String>,
    query: String,
    /// Set when the columns or the SQL changed, so the next reconcile reclassifies them.
    roles_stale: bool,
    /// The camera's zoom, kept because `render` has no access to `NodeContext` and the table's
    /// row height and column widths are in pixels, which the rem scope does not scale.
    zoom: f64,
    pub(super) search_open: bool,
    pub(super) search_input: Entity<InputState>,
    pub(super) search_query: String,
    /// Dropping it cancels the pending search, so reassigning *is* restart-the-debounce.
    search_task: Task<()>,
    /// Element ids derived from the node id. Built once: they never change, and `format!`ing
    /// five of them per frame per visible result is allocation for a constant.
    pub(super) ids: ElementIds,
    _subscriptions: [gpui_kit::Subscription; 3],
}

/// The node-scoped element ids a result node needs, minted once in [`ResultTable::new`].
#[derive(Debug, Clone)]
pub(super) struct ElementIds {
    pub(super) table: SharedString,
    pub(super) delete: SharedString,
    pub(super) pivot: SharedString,
    pub(super) search: SharedString,
    pub(super) search_close: SharedString,
    pub(super) detail_close: SharedString,
}

impl ElementIds {
    fn new(node: &NodeId) -> Self {
        Self {
            table: SharedString::from(format!("{node}-table")),
            delete: SharedString::from(format!("{node}-delete")),
            pivot: SharedString::from(format!("{node}-pivot")),
            search: SharedString::from(format!("{node}-search")),
            search_close: SharedString::from(format!("{node}-search-close")),
            detail_close: SharedString::from(format!("{node}-detail-close")),
        }
    }
}

impl ResultTable {
    /// Pulls the node's current rows, widths and zoom into the table.
    ///
    /// Called from `body` each frame, as `QueryEditor::reconcile` is, because the document is
    /// the source of truth and the entity outlives any one frame's view of it.
    fn reconcile(&mut self, incoming: &Incoming, cx: &mut Context<Self>) {
        self.zoom = incoming.zoom;
        if self.query != incoming.query {
            self.query = incoming.query.to_string();
            let facts = QueryFacts::of(&self.query);
            self.tables = facts.tables;
            self.editable = facts.editable;
            self.roles_stale = true;
        }
        let adopted = self.table.update(cx, |table, cx| {
            // The transposition changes what every row and column index means, so the table has
            // to re-read its columns before anything else this frame is measured against them.
            if table.delegate_mut().set_pivoted(incoming.pivoted) {
                table.refresh(cx);
            }
            let adopted = table.delegate_mut().adopt(
                &incoming.rows,
                incoming.widths,
                (incoming.available, incoming.zoom),
            );
            if adopted.needs_refresh() {
                // `column()` is only read on prepare and refresh, so a width the delegate just
                // recomputed — or a zoom that changed what it converts to in pixels — is
                // invisible until the table is told to re-read it.
                table.refresh(cx);
            }
            adopted
        });
        // Roles are classified against the *columns*, which only a new result can change. A
        // zoom moves the pixel widths and nothing else, so it must not drag a schema walk and
        // a notify along behind it.
        if adopted.rows_changed || self.roles_stale {
            self.refresh_roles(cx);
            cx.notify();
        } else if adopted.needs_refresh() {
            cx.notify();
        }
    }

    /// Classifies the columns against the schema the language server holds.
    ///
    /// Recomputed only when the columns or the SQL change: the schema is filled once a
    /// connection answers and does not move afterwards, and walking every foreign key per frame
    /// would be pure waste.
    fn refresh_roles(&mut self, cx: &mut Context<Self>) {
        self.roles_stale = false;
        let tables: Vec<String> = self.tables.iter().map(ToString::to_string).collect();
        let shared = crate::node::query::language::SqlLanguage::schema(cx);
        self.table.update(cx, |table, _| {
            let columns: Vec<String> = table
                .delegate()
                .result_rows()
                .columns()
                .iter()
                .map(|column| column.name.clone())
                .collect();
            let schema = shared.read();
            let roles = column_roles::classify(&columns, &tables, Some(&schema));
            table.delegate_mut().set_roles(roles);
        });
    }

    /// The toolbar's Pivot button. It selects its own node first, because the handler pivots the
    /// *selection* — a click on an unselected result would otherwise pivot whichever node was.
    /// Dispatching rather than calling the toggle keeps the button, the palette and the keyboard
    /// on one path.
    fn dispatch_pivot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node = self.node.clone();
        self.document.update(cx, |document, cx| {
            document.select_only([node]);
            cx.notify();
        });
        window.dispatch_action(Box::new(crate::commands::actions::result::Pivot), cx);
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = false;
        self.search_query.clear();
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.apply_search(cx);
        cx.notify();
    }

    /// Restarts the debounce. Matching every keystroke against every cell of a 6,515-row result
    /// would make typing stutter; `useResultSearchMatches` waits the same 100 ms.
    fn on_search_changed(&mut self, text: String, cx: &mut Context<Self>) {
        self.search_query = text;
        self.search_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, ResultTable::apply_search).ok();
        });
    }

    fn apply_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query.clone();
        self.table.update(cx, |table, cx| {
            let matches = search::search(table.delegate().result_rows(), &query);
            table.delegate_mut().set_matches(matches);
            table.refresh(cx);
        });
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn deletable_rows_for_test(&self, cx: &App) -> Option<usize> {
        self.deletable_rows(cx)
    }

    #[cfg(test)]
    pub(crate) fn is_search_open(&self) -> bool {
        self.search_open
    }

    /// Opens the find bar and sets its text, as typing into it would.
    #[cfg(test)]
    pub(crate) fn set_search_for_test(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_open = true;
        self.search_input
            .update(cx, |input, cx| input.set_value(query, window, cx));
        // `set_value` does not emit `InputEvent::Change`, so drive the same entry point typing
        // would — debounce included.
        self.on_search_changed(query.to_string(), cx);
    }

    /// Runs the queries a followed reference produced, with this result as their source so the
    /// new nodes stack under it and the edges say where they came from.
    fn follow_reference(&mut self, queries: Vec<String>, cx: &mut Context<Self>) {
        let document = self.document.clone();
        let node = self.node.clone();
        crate::execution::run_queries(&document, &node, queries, cx);
    }

    fn close_detail(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_detail(None);
            cx.notify();
        });
        cx.notify();
    }

    /// Escape clears, and so does a press on the table's blank space.
    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.table.read(cx).delegate().editing().is_some() {
            self.cancel_edit(cx);
            return;
        }
        if self.table.read(cx).delegate().detail().is_some() {
            self.close_detail(cx);
            return;
        }
        if self.search_open {
            // Escape closes the find bar first, as it does in the reference; a second escape
            // then reaches the canvas and clears the node selection.
            let cleared = !self.search_query.is_empty() || self.search_open;
            self.search_open = false;
            self.search_query.clear();
            self.apply_search(cx);
            cx.notify();
            if cleared {
                return;
            }
        }
        let cleared = self.table.update(cx, |table, cx| {
            let cleared = table.delegate_mut().selection_mut().clear();
            if cleared {
                cx.notify();
            }
            cleared
        });
        if cleared {
            cx.notify();
        }
    }

    /// `cmd-c` copies the selection as TSV, which pastes cleanly into a spreadsheet.
    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let table = self.table.read(cx);
        let delegate = table.delegate();
        let Some(text) = selection::to_tsv(
            delegate.result_rows(),
            delegate.selection(),
            delegate.matches().visible(),
        ) else {
            return;
        };
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
    }

    /// `TableEvent::ColumnWidthsChanged` carries **every** column's width in pixels, and only on
    /// drag end — which is exactly when the reference commits, so a drag writes one undo entry
    /// rather than one per frame.
    fn on_widths_changed(&mut self, widths: &[gpui_kit::Pixels], cx: &mut Context<Self>) {
        let scale = self.table.read(cx).delegate().scale();
        if scale <= 0.0 {
            return;
        }
        let names: Vec<String> = self
            .table
            .read(cx)
            .delegate()
            .column_names()
            .map(ToString::to_string)
            .collect();
        let resolved: BTreeMap<String, f64> = names
            .into_iter()
            .zip(widths)
            .map(|(name, width)| (name, f64::from(f32::from(*width)) / scale))
            .collect();
        if resolved.is_empty() {
            return;
        }

        let node = self.node.clone();
        self.document.update(cx, |document, cx| {
            if document.update_data::<ResultData>(&node, |data| {
                data.column_widths = Some(resolved);
            }) {
                // A drag is one deliberate edit, so seal it rather than letting the next one
                // coalesce into it.
                document.checkpoint();
                cx.notify();
            }
        });
    }
}

impl ResultState {
    pub(crate) fn new(
        id: &NodeId,
        data: &ResultData,
        document: &Entity<Document>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let rows = document.read(cx).result(id).cloned().unwrap_or_default();
        let edit_input = cx.new(|cx| InputState::new(window, cx));
        let delegate = ResultDelegate::new(rows, data.column_widths.as_ref(), edit_input.clone());
        let table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                // The reference has no sorting or column reordering — clicking a header selects
                // the column there — so both stay off and a header press is free for selection.
                .sortable(false)
                .col_movable(false)
                .row_header(false)
                // Its own selection is a single `Option<(row, col)>`; the rectangle, the row
                // bands and their mutual exclusivity live in `selection.rs`, and leaving both on
                // would paint two disagreeing selections.
                .cell_selectable(false)
                .row_selectable(false)
                .col_selectable(false)
        });
        let node = id.clone();
        let document = document.clone();
        let search_input = toolbar::search_input(window, cx);
        let query = data.query.clone();
        let facts = QueryFacts::of(&query);
        let ids = ElementIds::new(id);
        let inner = cx.new(|cx| ResultTable {
            _subscriptions: [
                cx.subscribe(
                    &table,
                    |this: &mut ResultTable, _, event: &TableEvent, cx| {
                        if let TableEvent::ColumnWidthsChanged(widths) = event {
                            this.on_widths_changed(widths, cx);
                        }
                    },
                ),
                cx.subscribe(
                    &edit_input,
                    |this: &mut ResultTable, _, event: &InputEvent, cx| {
                        // Enter commits a single-line edit; the reference uses it for the bool
                        // picker and a modifier elsewhere, but every in-cell field here is one
                        // line, so Enter is unambiguous.
                        if matches!(event, InputEvent::PressEnter { .. }) {
                            this.commit_edit(cx);
                        }
                    },
                ),
                cx.subscribe(
                    &search_input,
                    |this: &mut ResultTable, input, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            let text = input.read(cx).value().to_string();
                            this.on_search_changed(text, cx);
                        }
                    },
                ),
            ],
            tables: facts.tables,
            editable: facts.editable,
            ids,
            query,
            roles_stale: true,
            zoom: 1.0,
            table,
            node,
            document,
            search_open: false,
            search_input,
            search_query: String::new(),
            search_task: Task::ready(()),
        });
        let weak = inner.downgrade();
        inner.update(cx, |this, cx| {
            this.table
                .update(cx, |table, _| table.delegate_mut().set_owner(weak));
            let _ = cx;
        });
        Self { inner }
    }

    #[cfg(test)]
    pub(crate) fn inner(&self) -> Entity<ResultTable> {
        self.inner.clone()
    }

    #[cfg(test)]
    pub(crate) fn table(&self, cx: &App) -> Entity<TableState<ResultDelegate>> {
        self.inner.read(cx).table.clone()
    }
}

/// What `body` hands the entity each frame.
///
/// Borrowed rather than owned: this is built once per visible result per frame, and the two
/// owned fields it used to carry were a `BTreeMap` and a `String` cloned for a comparison that
/// almost always says "unchanged".
struct Incoming<'a> {
    rows: Arc<ResultSet>,
    widths: Option<&'a BTreeMap<String, f64>>,
    query: &'a str,
    available: f64,
    zoom: f64,
    pivoted: bool,
}

pub(crate) fn body(
    id: &NodeId,
    data: &ResultData,
    context: NodeContext<'_>,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(NodeState::Result(state)) = context.state else {
        return div().into_any_element();
    };
    let Some(rows) = context.document.read(cx).result(id).cloned() else {
        return empty_state("Run the query to load rows", cx);
    };

    let incoming = Incoming {
        rows,
        widths: data.column_widths.as_ref(),
        query: &data.query,
        available: context.size.width,
        zoom: context.zoom,
        pivoted: data.pivoted.unwrap_or_default(),
    };
    let inner = state.inner.clone();
    inner.update(cx, |table, cx| table.reconcile(&incoming, cx));
    inner.into_any_element()
}

impl Render for ResultTable {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let table = self.table.clone();
        let empty = table.read(cx).delegate().result_rows().is_empty();

        // Announces what is picked out, which is also the only stable observable a test has for
        // the selection: `DataTable` registers no ids of its own for cells.
        let selected = table.read(cx).delegate().selection_summary().map_or_else(
            || "no selection".to_string(),
            |what| format!("{what} selected"),
        );

        #[allow(
            clippy::cast_possible_truncation,
            reason = "a row height in pixels is far inside f32"
        )]
        let row_height = px((ROW_HEIGHT * self.zoom) as f32);

        div()
            .id(self.ids.table.clone())
            .aria_label(SharedString::from(selected))
            .test_support()
            // Sits above the table on the focus path, so these fire while the table holds focus
            // — the same trick `QueryNode` uses for `Query::Format`.
            .key_context(crate::commands::RESULT_NODE)
            .on_action(
                cx.listener(|this, _: &crate::commands::actions::tool::Select, _, cx| {
                    this.clear_selection(cx);
                }),
            )
            .on_action(cx.listener(
                |this, _: &crate::commands::actions::edit::copy::Copy, _, cx| {
                    this.copy_selection(cx);
                },
            ))
            .on_action(cx.listener(
                |this, _: &crate::commands::actions::page::Search, window, cx| {
                    // The find bar filters rows, which the record view does not lay out as rows.
                    // Letting the action carry on hands the same key to the page-wide search,
                    // which is what the reference's `selected && !pivoted` gate amounts to.
                    if this.table.read(cx).delegate().is_pivoted() {
                        cx.propagate();
                        return;
                    }
                    this.open_search(window, cx);
                },
            ))
            .v_flex()
            .size_full()
            .overflow_hidden()
            .on_scroll_wheel(absorb_scroll(table.clone()))
            // A press on the body but not on a cell clears the selection, as clicking blank
            // space inside the scroll container does in the reference. A cell claims its own
            // press first.
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.clear_selection(cx)),
            )
            .child(self.toolbar(cx))
            .children(self.edit_error(cx))
            .children(self.detail_pane(cx))
            .child(if empty {
                empty_state("No results", cx)
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        DataTable::new(&table)
                            .bordered(false)
                            .stripe(false)
                            .with_size(Size::Size(row_height)),
                    )
                    .into_any_element()
            })
    }
}

/// The tables a query reads, for the toolbar's badges.
/// Everything the toolbar and the edit path need to know about the SQL behind a result, from
/// one tree-sitter parse.
///
/// They used to parse separately — the badges here, the writable-table check in `edit.rs` — and
/// the second one sat on the render path, so a connected result re-parsed its statement on every
/// frame. One parse, cached against the SQL, answers both.
struct QueryFacts {
    tables: Vec<SharedString>,
    editable: Option<String>,
}

impl QueryFacts {
    fn of(query: &str) -> Self {
        let info = peek_lsp::analyze_query(query);
        Self {
            editable: editable::editable_table(&info),
            tables: info
                .tables
                .into_iter()
                .map(|table| SharedString::from(table.name))
                .collect(),
        }
    }
}

/// Stops a wheel the table actually consumed from also panning the canvas.
///
/// `CanvasElement` registers its wheel listener before the node elements precisely so a node can
/// absorb a scroll, but it only works if the node says it did: gpui's built-in scroll handling
/// does not stop propagation, and `DataTable` adds none of its own. Without this, scrolling a
/// result scrolls the rows *and* drags the whole canvas with them.
///
/// The rule is `useScrollFallthrough`'s `canAbsorb`: absorb only while there is still room to
/// move in the direction being scrolled, so reaching the end of the rows hands the gesture back
/// and the canvas starts panning — which is what makes a table inside an infinite canvas feel
/// like part of it rather than a trap.
fn absorb_scroll(
    table: Entity<TableState<ResultDelegate>>,
) -> impl Fn(&gpui_kit::ScrollWheelEvent, &mut Window, &mut App) + 'static {
    move |event, window, cx| {
        // cmd/ctrl + wheel is a zoom; the canvas takes it in the capture phase and never
        // reaches here, but bailing out matches the reference's own `ctrlKey` guard.
        if event.modifiers.secondary() || event.modifiers.control {
            return;
        }
        let delta = event.delta.pixel_delta(window.line_height()).y;
        if delta == gpui_kit::px(0.0) {
            return;
        }
        let scroll = table
            .read(cx)
            .vertical_scroll_handle
            .0
            .borrow()
            .base_handle
            .clone();
        if has_room_to_scroll(delta, scroll.offset().y, scroll.max_offset().y) {
            cx.stop_propagation();
        }
    }
}

/// Whether a wheel of `delta` still has somewhere to go, given where the rows are scrolled to.
///
/// gpui's scroll offset runs from `0` at the top to `-max` at the bottom, and a **positive**
/// delta moves the content down — that is, scrolling up. Split out from the handler because it
/// is the whole rule, and a handler needing a window and an entity is not something a test can
/// pin.
fn has_room_to_scroll(
    delta: gpui_kit::Pixels,
    offset: gpui_kit::Pixels,
    max: gpui_kit::Pixels,
) -> bool {
    const EDGE: f32 = 0.5;
    if max <= gpui_kit::px(EDGE) {
        // Nothing to scroll: every wheel belongs to the canvas.
        return false;
    }
    if delta > gpui_kit::px(0.0) {
        offset < gpui_kit::px(-EDGE)
    } else {
        -offset < max - gpui_kit::px(EDGE)
    }
}

/// `ResultEmpty.tsx`: centred, muted, one line.
fn empty_state(message: &str, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    div()
        .size_full()
        .v_flex()
        .items_center()
        .justify_center()
        .p(rems(1.0))
        .text_size(rems(0.75))
        .text_color(theme.fg_muted)
        .child(message.to_string())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use peek_document::ResultData;

    use super::title;

    fn data(query: &str) -> ResultData {
        ResultData {
            query: query.to_string(),
            ..ResultData::default()
        }
    }

    #[test]
    fn the_title_is_the_querys_first_meaningful_line() {
        assert_eq!(
            title(&data("\n\n  select * from users  ")),
            "select * from users"
        );
    }

    /// `nodeHeading` strips a leading comment marker, so a documented query is not titled `--`.
    #[test]
    fn a_leading_comment_marker_is_stripped() {
        assert_eq!(title(&data("-- everyone\nselect 1")), "everyone");
    }

    #[test]
    fn a_long_query_is_cut_with_an_ellipsis() {
        let long = format!("select {}", "x".repeat(100));
        let title = title(&data(&long));
        assert_eq!(title.chars().count(), 63, "60 characters plus the ellipsis");
        assert!(title.ends_with("..."));
    }

    #[test]
    fn an_empty_query_still_has_a_title() {
        assert_eq!(title(&data("")), "result");
    }

    /// Multi-byte text must be cut on a character boundary, not a byte one.
    #[test]
    fn a_long_multibyte_query_does_not_panic() {
        let long = "é".repeat(200);
        assert_eq!(title(&data(&long)).chars().count(), 63);
    }
}

/// Rendering tests, which need a window.
///
/// `DataTable`'s internals register no ids a test can reach, so these assert on `TableState`
/// instead — `dump_range` and `visible_range` are the discriminating observables, and
/// `visible_range` is the only way to prove the table is actually virtualising.
#[cfg(test)]
mod render_tests {
    use gpui_kit::component::Root;
    use gpui_kit::component::table::TableEvent;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
        PinchEvent, SharedString, TestAppContext, TouchPhase, VisualTestContext, point, px, size,
    };
    use peek_document::{CanvasDocument, Cell, Column, NodeData, NodeId, ResultSet};

    use crate::workspace::WorkspaceView;

    const DOCUMENT: &str = r#"{
      "version": 1,
      "activePageId": "page_test0001",
      "pageOrder": ["page_test0001"],
      "pages": {
        "page_test0001": {
          "id": "page_test0001",
          "name": "Page 1",
          "nodes": [{
            "id": "query_aaaaaaaa-result-0",
            "type": "result",
            "position": { "x": 40, "y": 120 },
            "width": 600,
            "height": 440,
            "data": { "query": "select * from users" }
          }],
          "edges": [],
          "viewport": { "x": 0, "y": 0, "zoom": 1 }
        }
      }
    }"#;

    fn result_node() -> NodeId {
        NodeId::from("query_aaaaaaaa-result-0")
    }

    fn rows(count: usize) -> ResultSet {
        ResultSet::new(
            vec![Column::new("id", "INT4"), Column::new("name", "VARCHAR")],
            (0..count)
                .map(|index| {
                    vec![
                        Cell::Int(i64::try_from(index).unwrap()),
                        Cell::Text(format!("row {index}")),
                    ]
                })
                .collect(),
        )
    }

    /// Opens a workspace holding one result node, with `count` rows already loaded — the state a
    /// document plus its sidecar arrives in.
    fn open(
        cx: &mut TestAppContext,
        count: usize,
    ) -> (
        gpui_kit::WindowHandle<Root>,
        gpui_kit::Entity<WorkspaceView>,
    ) {
        open_zoomed(cx, count, 1.0)
    }

    /// The same, with the document's saved viewport at `zoom`, which is how a page reopens at
    /// whatever the camera was left at — including far enough out to reduce its nodes.
    fn open_zoomed(
        cx: &mut TestAppContext,
        count: usize,
        zoom: f64,
    ) -> (
        gpui_kit::WindowHandle<Root>,
        gpui_kit::Entity<WorkspaceView>,
    ) {
        let document_json = DOCUMENT.replace(r#""zoom": 1"#, &format!(r#""zoom": {zoom}"#));
        cx.update(|cx| {
            let mut config = peek_config::PeekConfig::default();
            config.theme = peek_config::ThemeId::Midday;
            crate::init(&config, cx);
        });
        let mut workspace = None;
        let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
            let document = CanvasDocument::from_json(&document_json).unwrap();
            let view = cx.new(|cx| {
                let view = WorkspaceView::with_document("test", document, window, cx);
                view.document(cx).update(cx, |document, _| {
                    document.set_result(result_node(), rows(count));
                });
                view
            });
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        (handle, workspace.unwrap())
    }

    /// Reads the table's own view of its contents, through the node state the canvas holds.
    fn dump(
        cx: &mut TestAppContext,
        workspace: &gpui_kit::Entity<WorkspaceView>,
        range: std::ops::Range<usize>,
    ) -> (Vec<String>, Vec<Vec<String>>) {
        cx.update(|cx| {
            let table = workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .expect("the result node has a table");
            table.read(cx).dump_range(range, cx)
        })
    }

    fn table_bounds(
        cx: &mut TestAppContext,
        handle: gpui_kit::WindowHandle<Root>,
    ) -> gpui_kit::Bounds<gpui_kit::Pixels> {
        cx.update_window(handle.into(), |_, window, _| {
            window
                .find(SharedString::from(format!("{}-table", result_node())))
                .bounds()
        })
        .unwrap()
    }

    /// A press and release at one point. `Window::click` targets an element by id, which would
    /// sidestep the very dispatch these tests are about.
    fn click_at(visual: &mut VisualTestContext, at: gpui_kit::Point<gpui_kit::Pixels>) {
        visual.simulate_event(MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Modifiers::default(),
        });
        visual.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        visual.run_until_parked();
    }

    /// A press, a move with the button down, and a release.
    fn drag_cells(
        visual: &mut VisualTestContext,
        from: gpui_kit::Point<gpui_kit::Pixels>,
        to: gpui_kit::Point<gpui_kit::Pixels>,
    ) {
        visual.simulate_event(MouseMoveEvent {
            position: from,
            pressed_button: None,
            modifiers: Modifiers::default(),
        });
        visual.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: from,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.simulate_event(MouseMoveEvent {
            position: to,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        });
        visual.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: to,
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        visual.run_until_parked();
    }

    fn double_click_at(visual: &mut VisualTestContext, at: gpui_kit::Point<gpui_kit::Pixels>) {
        click_at(visual, at);
        visual.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 2,
        });
        visual.run_until_parked();
    }

    fn shift_click_at(visual: &mut VisualTestContext, at: gpui_kit::Point<gpui_kit::Pixels>) {
        let modifiers = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        visual.simulate_event(MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers,
        });
        visual.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers,
            click_count: 1,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers,
            click_count: 1,
        });
        visual.run_until_parked();
    }

    /// Opens the find bar, types, and waits out the 100 ms debounce.
    fn type_search(
        cx: &mut TestAppContext,
        handle: gpui_kit::WindowHandle<Root>,
        workspace: &gpui_kit::Entity<WorkspaceView>,
        query: &str,
    ) {
        cx.update_window(handle.into(), |_, window, cx| {
            let table = workspace
                .read(cx)
                .result_inner(&result_node(), cx)
                .expect("the result node has a table");
            table.update(cx, |this, cx| this.set_search_for_test(query, window, cx));
        })
        .unwrap();
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    #[gpui_kit::test]
    fn the_table_shows_the_querys_columns_and_rows(cx: &mut TestAppContext) {
        let (_handle, workspace) = open(cx, 3);
        let (headers, body) = dump(cx, &workspace, 0..3);
        assert_eq!(headers, ["id", "name"]);
        assert_eq!(body.len(), 3);
        assert_eq!(body[0], ["0", "row 0"]);
        assert_eq!(body[2], ["2", "row 2"]);
    }

    /// Re-running a query replaces the rows, and the table has to pick them up: the delegate
    /// adopts on render, so without that it would keep showing the previous run's results.
    ///
    /// NULL copies as the empty string, so a spreadsheet paste leaves the cell blank rather than
    /// spelling out "null" — `stringifyValue`'s rule, which the export path shares.
    #[gpui_kit::test]
    fn re_running_replaces_the_rows_and_null_reads_as_empty(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 3);
        assert_eq!(dump(cx, &workspace, 0..3).1.len(), 3);

        cx.update(|cx| {
            workspace.read(cx).document(cx).update(cx, |document, _| {
                document.set_result(
                    result_node(),
                    ResultSet::new(vec![Column::new("only", "INT4")], vec![vec![Cell::Null]]),
                );
            });
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();

        let (headers, body) = dump(cx, &workspace, 0..1);
        assert_eq!(headers, ["only"], "the new columns replaced the old");
        assert_eq!(body.len(), 1);
        assert_eq!(body[0], [""], "NULL copies as the empty string");
    }

    /// What the table says is selected. `DataTable` registers no ids for its cells, so the
    /// accessible label on the body is the observable — and it is the same string a screen
    /// reader gets.
    fn selection_label(cx: &mut TestAppContext, handle: gpui_kit::WindowHandle<Root>) -> String {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window
                .find(SharedString::from(format!("{}-table", result_node())))
                .label()
                .unwrap_or_default()
                .to_string()
        })
        .unwrap()
    }

    fn cell_at(
        bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
        row: f32,
        column: f32,
    ) -> gpui_kit::Point<gpui_kit::Pixels> {
        // The header is one row tall, so row 0 starts one row height down.
        // Two columns share the 598px body. Down the page: the toolbar (28), the column header
        // (34), then the rows, and half a row to land in the middle of one.
        point(
            bounds.origin.x + px(40.0) + px(column * 299.0),
            bounds.origin.y + px(28.0 + 34.0 + 17.0) + px(row * 34.0),
        )
    }

    /// A full click on a cell selects that cell, and the node with it.
    ///
    /// Worth pinning because it is easy to assume the opposite: the canvas registers its
    /// window-level mouse listeners so they run *before* any node element, and it does resolve
    /// this press as a node-body hit. It does not stop propagation, though, so the cell's own
    /// handler still runs — clicking a cell selects the cell *and* the node, which is what React
    /// Flow does too.
    #[gpui_kit::test]
    fn clicking_a_cell_selects_the_cell_and_the_node(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        assert_eq!(selection_label(cx, handle), "no selection");

        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(&mut visual, cell_at(bounds, 2.0, 0.0));

        assert_eq!(selection_label(cx, handle), "1 cell selected");
        assert!(
            cx.update(|cx| workspace
                .read(cx)
                .document(cx)
                .read(cx)
                .is_selected(&result_node())),
            "and the node is selected too, as a body press has always done"
        );
    }

    /// Dragging from one cell to another selects the rectangle between them.
    #[gpui_kit::test]
    fn dragging_across_cells_selects_a_rectangle(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);

        drag_cells(
            &mut visual,
            cell_at(bounds, 0.0, 0.0),
            cell_at(bounds, 2.0, 1.0),
        );
        assert_eq!(
            selection_label(cx, handle),
            "6 cells selected",
            "three rows by two columns"
        );
    }

    /// Shift is the row gesture, and it replaces a cell rectangle rather than coexisting with
    /// one — a copy has to mean one unambiguous thing.
    #[gpui_kit::test]
    fn shift_clicking_selects_a_row_and_replaces_a_cell_selection(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);

        click_at(&mut visual, cell_at(bounds, 1.0, 0.0));
        assert_eq!(selection_label(cx, handle), "1 cell selected");

        shift_click_at(&mut visual, cell_at(bounds, 3.0, 0.0));
        assert_eq!(selection_label(cx, handle), "1 row selected");
    }

    /// Installs a schema where `orders.user_id` points at `users.id`.
    fn with_references(cx: &mut TestAppContext) {
        use std::collections::HashMap;

        cx.update(|cx| {
            let mut tables = HashMap::new();
            tables.insert(
                "orders".to_string(),
                vec![("user_id".to_string(), "int4".to_string())],
            );
            let mut references = HashMap::new();
            references.insert("users.id".to_string(), vec!["orders.user_id".to_string()]);
            let index = peek_lsp::SchemaIndex::from_raw(tables, references, HashMap::new());
            peek_lsp::set_schema(
                &crate::node::query::language::SqlLanguage::schema(cx),
                index,
            );
        });
    }

    /// Clicking a reference asks the database what it points at and puts the answer on the
    /// canvas. Without a connection there is nothing to ask, so this pins the two halves that do
    /// not need one: the cell is a link, and pressing it does not select instead.
    #[gpui_kit::test]
    fn a_reference_cell_is_a_link_and_does_not_select(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 8);
        with_references(cx);
        cx.update(|cx| {
            workspace.read(cx).document(cx).update(cx, |document, _| {
                document.update_data::<peek_document::ResultData>(&result_node(), |data| {
                    data.query = "select total, user_id from orders".to_string();
                });
                document.set_result(
                    result_node(),
                    ResultSet::new(
                        vec![Column::new("total", "INT4"), Column::new("user_id", "INT4")],
                        vec![vec![Cell::Int(5), Cell::Int(7)]],
                    ),
                );
            });
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();

        let linked = cx.update(|cx| {
            workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .map(|table| table.read(cx).delegate().follows_references(1))
        });
        assert_eq!(linked, Some(true), "the reference column is followable");

        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(&mut visual, cell_at(bounds, 0.0, 1.0));
        assert_eq!(
            selection_label(cx, handle),
            "no selection",
            "a press on a link belongs to the link, not to cell selection"
        );
    }

    /// A column the schema says nothing about is not a link, so an ordinary cell still selects.
    ///
    /// This is why a `*_id` column with no schema behind it is tinted but inert: there is no
    /// target to follow, only a name that looks like one.
    #[gpui_kit::test]
    fn a_column_with_no_known_target_is_not_a_link(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 8);
        let linked = cx.update(|cx| {
            workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .map(|table| table.read(cx).delegate().follows_references(1))
        });
        assert_eq!(linked, Some(false));

        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(&mut visual, cell_at(bounds, 0.0, 1.0));
        assert_eq!(selection_label(cx, handle), "1 cell selected");
    }

    /// Double-clicking a short value opens an editor in the cell — and, crucially, does not
    /// abort the process on the way.
    ///
    /// The cell's own handler runs inside a `TableState` update, and opening an editor reads
    /// that same state back. Doing it there is a re-entrant borrow, which gpui turns into a
    /// non-unwinding panic: the window dies with it. `open_cell` defers to the next turn.
    ///
    /// This needs a connection, which is why the first version of the suite could not have
    /// caught it: every editing path bails before the read when there is nowhere to send an
    /// UPDATE.
    #[gpui_kit::test]
    fn double_clicking_a_cell_opens_an_editor_without_re_entering_the_table(
        cx: &mut TestAppContext,
    ) {
        let (handle, workspace) = open(cx, 8);
        cx.update(|cx| {
            crate::database::Database::mark_connected_for_test(cx);
            // A single-table SELECT, so the result reads as editable.
            workspace.read(cx).document(cx).update(cx, |document, _| {
                document.update_data::<peek_document::ResultData>(&result_node(), |data| {
                    data.query = "select id, name from peek_probe".to_string();
                });
            });
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();

        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        double_click_at(&mut visual, cell_at(bounds, 1.0, 1.0));
        visual.run_until_parked();

        let editing = cx.update(|cx| {
            workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .and_then(|table| {
                    table
                        .read(cx)
                        .delegate()
                        .editing()
                        .map(|edit| (edit.row, edit.column))
                })
        });
        assert_eq!(editing, Some((1, 1)), "the cell opened for editing");
    }

    /// Double-clicking a short value opens an editor in the cell.
    ///
    /// Nothing is editable without a connection, so this asserts the *refusal* — the commit path
    /// itself is covered by `peek_db::mutation`'s builder tests and `editable`'s refusal tests.
    #[gpui_kit::test]
    fn a_cell_does_not_open_for_editing_without_a_connection(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 4);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        double_click_at(&mut visual, cell_at(bounds, 0.0, 0.0));

        let editing = cx.update(|cx| {
            workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .and_then(|table| table.read(cx).delegate().editing().map(|_| ()))
        });
        assert!(
            editing.is_none(),
            "there is nowhere to send an UPDATE, so the cell stays read-only"
        );
    }

    /// The delete affordance only appears when there are rows selected in a result that can be
    /// written — the one irreversible action must never sit there inviting a stray click.
    #[gpui_kit::test]
    fn delete_is_not_offered_without_a_connection(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 4);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        shift_click_at(&mut visual, cell_at(bounds, 1.0, 0.0));

        let offered = cx.update(|cx| {
            workspace
                .read(cx)
                .result_inner(&result_node(), cx)
                .and_then(|table| table.read(cx).deletable_rows_for_test(cx))
        });
        assert_eq!(offered, None);
    }

    /// A JSON cell shows a one-line summary in the grid; double-clicking opens its full tree in
    /// the detail pane, which is the only way to read it.
    #[gpui_kit::test]
    fn double_clicking_a_json_cell_opens_its_value(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 4);
        cx.update(|cx| {
            workspace.read(cx).document(cx).update(cx, |document, _| {
                document.set_result(
                    result_node(),
                    ResultSet::new(
                        vec![Column::new("id", "INT4"), Column::new("meta", "JSONB")],
                        vec![vec![
                            Cell::Int(1),
                            Cell::Json(serde_json::json!({"plan": "pro", "seats": 4})),
                        ]],
                    ),
                );
            });
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();

        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        double_click_at(&mut visual, cell_at(bounds, 0.0, 1.0));

        let open = cx.update(|cx| {
            workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .and_then(|table| table.read(cx).delegate().detail())
        });
        assert_eq!(open, Some((0, 1)), "the JSON cell opened");
    }

    /// `cmd-f` opens the find bar while the table has focus.
    /// Pivoting transposes the grid: a table row is now one of the result's *columns*, and a
    /// table column is one record. `DataTable` registers no ids for its cells, so the delegate's
    /// own view of its contents is the observable.
    #[gpui_kit::test]
    fn pivoting_turns_the_columns_into_rows(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 2);
        assert_eq!(dump(cx, &workspace, 0..2).0, ["id", "name"]);

        cx.update_window(handle.into(), |_, window, cx| {
            workspace.read(cx).document(cx).update(cx, |document, cx| {
                super::pivot::toggle(document, &result_node());
                cx.notify();
            });
            window.render_frame(cx);
        })
        .unwrap();

        let (headers, body) = dump(cx, &workspace, 0..2);
        assert_eq!(headers, ["Field", "#1", "#2"]);
        assert_eq!(body[0], ["id", "0", "1"]);
        assert_eq!(body[1], ["name", "row 0", "row 1"]);
    }

    /// `Page::Search` is bound page-wide and the result node handles it too, so one keypress
    /// reaches two handlers. Only the deeper one may run: gpui clears `propagate_event` before
    /// **every** bubble-phase action listener (`window.rs`, "Actions stop propagation by default
    /// during the bubble phase"), so the node wins without asking. This is the assertion that
    /// pins it — its twin below is the same key with the node deliberately propagating.
    #[gpui_kit::test]
    fn cmd_f_in_a_focused_result_does_not_also_open_the_page_search(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(&mut visual, cell_at(bounds, 0.0, 0.0));

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("cmd-f", cx);
            window.render_frame(cx);
        })
        .unwrap();

        let opened = cx.update(|cx| {
            workspace
                .read(cx)
                .result_inner(&result_node(), cx)
                .is_some_and(|table| table.read(cx).is_search_open())
        });
        assert!(opened, "the in-result find bar opened");

        let page_search = cx
            .update_window(handle.into(), |_, window, _| {
                window.try_find("page-search").is_some()
            })
            .unwrap();
        assert!(!page_search, "and the page-wide search did not");
    }

    /// The find bar filters rows, which the record view has none of — so the same key belongs to
    /// the page-wide search while a result is pivoted. The twin of the test above: the one
    /// handler that *does* call `cx.propagate()` is the one that hands the key on.
    #[gpui_kit::test]
    fn cmd_f_does_not_open_the_find_bar_in_the_record_view(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(&mut visual, cell_at(bounds, 0.0, 0.0));

        cx.update_window(handle.into(), |_, window, cx| {
            workspace.read(cx).document(cx).update(cx, |document, cx| {
                super::pivot::toggle(document, &result_node());
                cx.notify();
            });
            window.render_frame(cx);
            window.press("cmd-f", cx);
            window.render_frame(cx);
        })
        .unwrap();

        let opened = cx.update(|cx| {
            workspace
                .read(cx)
                .result_inner(&result_node(), cx)
                .is_some_and(|table| table.read(cx).is_search_open())
        });
        assert!(!opened, "the find bar stayed shut");

        let page_search = cx
            .update_window(handle.into(), |_, window, _| {
                window.try_find("page-search").is_some()
            })
            .unwrap();
        assert!(page_search, "the page-wide search took the key instead");
    }

    #[gpui_kit::test]
    fn cmd_f_opens_the_find_bar(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        // The binding is on the node's own context, so the table has to hold focus first.
        click_at(&mut visual, cell_at(bounds, 0.0, 0.0));

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("cmd-f", cx);
            window.render_frame(cx);
        })
        .unwrap();

        let open = cx.update(|cx| {
            workspace
                .read(cx)
                .result_inner(&result_node(), cx)
                .is_some_and(|table| table.read(cx).is_search_open())
        });
        assert!(open, "the find bar opened");
    }

    /// Escape clears, which is the only way back to nothing without clicking elsewhere.
    #[gpui_kit::test]
    fn escape_clears_the_selection(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(&mut visual, cell_at(bounds, 1.0, 1.0));
        assert_eq!(selection_label(cx, handle), "1 cell selected");

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("escape", cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(selection_label(cx, handle), "no selection");
    }

    /// `cmd-c` puts the selection on the clipboard as TSV, which pastes cleanly into a
    /// spreadsheet. It is bound on the node's own context, so it only fires while the table
    /// holds focus — pressing into a cell is what gives it that focus.
    #[gpui_kit::test]
    fn copying_a_selection_writes_tab_separated_text(cx: &mut TestAppContext) {
        let (handle, _) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        drag_cells(
            &mut visual,
            cell_at(bounds, 0.0, 0.0),
            cell_at(bounds, 1.0, 1.0),
        );
        assert_eq!(selection_label(cx, handle), "4 cells selected");

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("cmd-c", cx);
            window.render_frame(cx);
        })
        .unwrap();

        let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(copied.as_deref(), Some("0\trow 0\n1\trow 1"));
    }

    /// New rows drop the selection: a position only means something against the ordering it was
    /// captured in, so keeping it would silently remap it onto different data.
    #[gpui_kit::test]
    fn re_running_the_query_drops_the_selection(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(&mut visual, cell_at(bounds, 1.0, 1.0));
        assert_eq!(selection_label(cx, handle), "1 cell selected");

        cx.update(|cx| {
            workspace.read(cx).document(cx).update(cx, |document, _| {
                document.set_result(result_node(), rows(5));
            });
        });
        assert_eq!(selection_label(cx, handle), "no selection");
    }

    /// Dragging a column edge resizes the column and must not drag the node with it.
    ///
    /// This is the one drag the table owns today. The table's header sits inside the node's
    /// **body** band — `NodeShell`'s own 32-unit header is above it — so the canvas resolves the
    /// press as a body hit and stays inert, which is what leaves the gesture to the resize
    /// handle. A press a few units higher would be the node's drag handle instead.
    #[gpui_kit::test]
    fn dragging_a_column_edge_resizes_it_without_moving_the_node(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        let bounds = table_bounds(cx, handle);

        let before = cx.update(|cx| {
            workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .map(|table| table.read(cx).dump(cx).0.len())
        });
        assert_eq!(before, Some(2), "two columns to begin with");

        let origin = cx.update(|cx| {
            workspace
                .read(cx)
                .document(cx)
                .read(cx)
                .node(&result_node())
                .map(|node| node.position)
        });

        // The boundary between the two columns, on the header row.
        let seam = point(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + px(10.0),
        );
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_event(MouseMoveEvent {
            position: seam,
            pressed_button: None,
            modifiers: Modifiers::default(),
        });
        visual.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: seam,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        visual.simulate_event(MouseMoveEvent {
            position: point(seam.x + px(60.0), seam.y),
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        });
        visual.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: point(seam.x + px(60.0), seam.y),
            modifiers: Modifiers::default(),
            click_count: 1,
        });
        visual.run_until_parked();

        let after = cx.update(|cx| {
            workspace
                .read(cx)
                .document(cx)
                .read(cx)
                .node(&result_node())
                .map(|node| node.position)
        });
        assert_eq!(
            after, origin,
            "the node stayed put; the drag belonged to the column"
        );
    }

    /// A finished column drag writes the new widths into the document, in **world** units.
    ///
    /// Without this they live only in the table's own state and are lost the next time the rows
    /// or the zoom change and the delegate recomputes from its defaults — and they would never
    /// reach `<connection>.json`, so the column would be back to its old width on reopen.
    ///
    /// Driven by emitting the event the table emits on drag end, because placing a synthetic
    /// pointer on a 4-pixel resize handle tests gpui-component's geometry rather than ours.
    #[gpui_kit::test]
    fn a_finished_column_drag_is_written_to_the_document(cx: &mut TestAppContext) {
        let (_handle, workspace) = open(cx, 5);

        cx.update(|cx| {
            let table = workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .expect("the result node has a table");
            table.update(cx, |_, cx| {
                cx.emit(TableEvent::ColumnWidthsChanged(vec![px(240.0), px(360.0)]));
            });
        });

        let widths = cx
            .update(|cx| {
                let document = workspace.read(cx).document(cx);
                let document = document.read(cx);
                peek_document::ResultData::get(&document.node(&result_node())?.kind)
                    .and_then(|data| data.column_widths.clone())
            })
            .expect("the drag persisted widths");

        // Zoom is 1 in the fixture, so world units and pixels agree.
        assert_eq!(widths.get("id").copied(), Some(240.0));
        assert_eq!(widths.get("name").copied(), Some(360.0));
    }

    /// Deleting a node whose table holds focus must not kill the canvas's key bindings.
    ///
    /// The table owns a focus handle, and a handle dies with its node — leaving gpui focused on
    /// nothing, which silently breaks every binding dispatched through the focus path. Before
    /// `CanvasView::reclaim_focus` this left `cmd-z` dead after clicking any cell.
    #[gpui_kit::test]
    fn deleting_a_focused_table_leaves_the_canvas_usable(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(
            &mut visual,
            point(bounds.origin.x + px(80.0), bounds.origin.y + px(120.0)),
        );

        let present = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                workspace
                    .read(cx)
                    .document(cx)
                    .read(cx)
                    .node(&result_node())
                    .is_some()
            })
        };

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("backspace", cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert!(!present(cx), "the node was deleted");

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("cmd-z", cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert!(
            present(cx),
            "undo still reaches the canvas after the focused node died"
        );
    }

    /// Typing into the find bar filters the rows to the matches, after the debounce.
    #[gpui_kit::test]
    fn searching_filters_the_rows_to_the_matches(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        // Row 7 is the only one whose name ends in "7" and is not also a prefix of another.
        type_search(cx, handle, &workspace, "row 17");

        let (_, body) = dump(cx, &workspace, 0..1);
        assert_eq!(body.len(), 1, "one row survived the search");
        assert_eq!(body[0], ["17", "row 17"]);
    }

    /// A search that matches nothing says so, and says something different from a query that
    /// returned nothing — the distinction the user needs.
    #[gpui_kit::test]
    fn a_search_with_no_matches_hides_every_row(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        type_search(cx, handle, &workspace, "zzzznothing");

        let shown = cx.update(|cx| {
            workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .map(|table| table.read(cx).delegate().matches().len())
        });
        assert_eq!(shown, Some(0));
    }

    /// Clearing the query brings every row back in its own order.
    #[gpui_kit::test]
    fn clearing_the_search_restores_every_row(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        type_search(cx, handle, &workspace, "row 17");
        type_search(cx, handle, &workspace, "");

        let (_, body) = dump(cx, &workspace, 0..3);
        assert_eq!(body.len(), 3);
        assert_eq!(body[0], ["0", "row 0"], "and in the result's own order");
    }

    /// Searching drops the selection: its positions were captured against the previous ordering,
    /// and search re-sorts rows by score.
    #[gpui_kit::test]
    fn searching_drops_a_selection_captured_before_it(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 20);
        let bounds = table_bounds(cx, handle);
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        click_at(&mut visual, cell_at(bounds, 1.0, 0.0));
        assert_eq!(selection_label(cx, handle), "1 cell selected");

        type_search(cx, handle, &workspace, "row 1");
        assert_eq!(selection_label(cx, handle), "no selection");
    }

    /// The whole point of the table: a result far larger than the node must not build a row per
    /// record. The real documents here reach 6,515 rows.
    #[gpui_kit::test]
    fn a_large_result_only_builds_the_rows_it_shows(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 6_515);
        VisualTestContext::from_window(handle.into(), cx).run_until_parked();

        let visible = cx.update(|cx| {
            let table = workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .expect("the result node has a table");
            table.read(cx).visible_range().rows().clone()
        });
        assert!(
            visible.len() < 100,
            "a 6,515-row result rendered {} rows",
            visible.len()
        );
        assert!(!visible.is_empty(), "but it does render some");
    }

    /// A result node whose query has not run holds no rows, and must say so rather than showing
    /// an empty grid that looks like a query returning nothing.
    #[gpui_kit::test]
    fn a_result_without_rows_does_not_build_a_table(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut config = peek_config::PeekConfig::default();
            config.theme = peek_config::ThemeId::Midday;
            crate::init(&config, cx);
        });
        let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
            let document = CanvasDocument::from_json(DOCUMENT).unwrap();
            let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    // ---- level of detail ----------------------------------------------------------------

    /// Past the point where a card is readable, the body stops being built: at this distance a
    /// 600x440 node is under 200 px wide, and the toolbar plus a few hundred cells inside it are
    /// laid out for nobody. The shell stays, so the node is still findable and still says what
    /// kind it is.
    #[gpui_kit::test]
    fn a_result_too_small_to_read_builds_no_table(cx: &mut TestAppContext) {
        let (handle, _workspace) = open_zoomed(cx, 200, 0.2);
        let (card, table) = cx
            .update_window(handle.into(), |_, window, _| {
                (
                    window
                        .try_find(SharedString::from(result_node().to_string()))
                        .is_some(),
                    window
                        .try_find(SharedString::from(format!("{}-table", result_node())))
                        .is_some(),
                )
            })
            .unwrap();
        assert!(card, "the shell must still draw, or the node vanishes");
        assert!(!table, "the table must not be built at this zoom");
    }

    /// The node the user is working with is exempt: it is what `Zoom::FitSelection` is about to
    /// fly to, and a selected card going blank under the selection ring reads as a bug.
    #[gpui_kit::test]
    fn a_selected_result_keeps_its_table_however_far_out_the_camera_is(cx: &mut TestAppContext) {
        let (handle, workspace) = open_zoomed(cx, 200, 0.2);
        cx.update(|cx| {
            workspace.read(cx).document(cx).update(cx, |document, cx| {
                document.select_only([result_node()]);
                cx.notify();
            });
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        let table = cx
            .update_window(handle.into(), |_, window, _| {
                window
                    .try_find(SharedString::from(format!("{}-table", result_node())))
                    .is_some()
            })
            .unwrap();
        assert!(table, "a selected result keeps its body");
    }

    /// Rows survive the round trip out and back: reducing a node must drop its *element tree*,
    /// never the state behind it.
    #[gpui_kit::test]
    fn a_reduced_result_still_has_its_rows_when_the_camera_comes_back(cx: &mut TestAppContext) {
        let (handle, workspace) = open(cx, 5);
        assert_eq!(dump(cx, &workspace, 0..5).1.len(), 5);

        // Driven through real pinches: `Page::viewport` only seeds the camera when the view is
        // built, so writing it back would leave the live camera exactly where it was.
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        let anchor = point(px(600.0), px(400.0));
        let pinch = |delta: f32, visual: &mut VisualTestContext| {
            for _ in 0..40 {
                visual.simulate_event(PinchEvent {
                    position: anchor,
                    delta,
                    modifiers: Modifiers::default(),
                    phase: TouchPhase::Moved,
                });
            }
        };
        pinch(-0.05, &mut visual);
        assert!(
            cx.update(|cx| workspace.read(cx).camera(cx).zoom) < peek_canvas::lod::REDUCE_BELOW,
            "the pinch has to actually cross the threshold for this to prove anything"
        );
        let reduced = cx
            .update_window(handle.into(), |_, window, _| {
                window
                    .try_find(SharedString::from(format!("{}-table", result_node())))
                    .is_some()
            })
            .unwrap();
        assert!(
            !reduced,
            "the body is gone while the camera is this far out"
        );

        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        pinch(0.06, &mut visual);
        assert_eq!(
            dump(cx, &workspace, 0..5).1.len(),
            5,
            "the rows must outlive a trip past the detail threshold"
        );
    }

    /// The reason zooming used to cost so much: a scale change moved the pixel widths, and the
    /// delegate treated that as new layout — re-measuring 30 rows of every column and restaling
    /// the schema roles — for widths that are world units and cannot have moved.
    #[gpui_kit::test]
    fn zooming_alone_never_re_resolves_the_column_widths(cx: &mut TestAppContext) {
        let (_handle, workspace) = open(cx, 50);
        let fresh = std::sync::Arc::new(rows(80));
        cx.update(|cx| {
            let table = workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .expect("the result node has a table");
            table.update(cx, |table, _| {
                let delegate = table.delegate_mut();
                let first = delegate.adopt(&fresh, None, (600.0, 1.0));
                assert!(
                    first.rows_changed,
                    "80 rows where there were 50 is new rows"
                );

                // The same rows, the same box, a different camera.
                let zoomed = delegate.adopt(&fresh, None, (600.0, 0.5));
                assert!(!zoomed.rows_changed, "a zoom does not change the rows");
                assert!(
                    !zoomed.widths_changed,
                    "a zoom does not change world widths"
                );
                assert!(zoomed.scale_changed, "but it does change the pixel scale");

                // And a frame that changes nothing at all reports nothing at all.
                let idle = delegate.adopt(&fresh, None, (600.0, 0.5));
                assert!(!idle.rows_changed && !idle.widths_changed && !idle.scale_changed);
            });
        });
    }

    /// A live query re-runs every ten seconds and usually gets identical rows back. That arrives
    /// as a *different* `Arc`, so the cheap pointer check alone would call it new rows and throw
    /// away the user's selection twice a minute.
    #[gpui_kit::test]
    fn a_live_re_run_with_identical_rows_is_not_new_rows(cx: &mut TestAppContext) {
        let (_handle, workspace) = open(cx, 50);
        let first = std::sync::Arc::new(rows(50));
        let identical = std::sync::Arc::new(rows(50));
        assert!(
            !std::sync::Arc::ptr_eq(&first, &identical),
            "the two runs must really be separate allocations"
        );
        cx.update(|cx| {
            let table = workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .expect("the result node has a table");
            table.update(cx, |table, _| {
                let delegate = table.delegate_mut();
                delegate.adopt(&first, None, (600.0, 1.0));
                let polled = delegate.adopt(&identical, None, (600.0, 1.0));
                assert!(
                    !polled.rows_changed,
                    "identical rows must not read as a new result"
                );
                assert!(!polled.widths_changed, "nor re-resolve the widths");

                // And the pointer must have been adopted anyway, or every later frame would
                // walk every cell again to reach the same answer.
                let next_frame = delegate.adopt(&identical, None, (600.0, 1.0));
                assert!(!next_frame.rows_changed && !next_frame.widths_changed);
            });
        });
    }

    /// A widened node is a real layout change, so the widths do have to be re-resolved.
    #[gpui_kit::test]
    fn resizing_the_node_does_re_resolve_them(cx: &mut TestAppContext) {
        let (_handle, workspace) = open(cx, 50);
        let shared = std::sync::Arc::new(rows(50));
        cx.update(|cx| {
            let table = workspace
                .read(cx)
                .result_table(&result_node(), cx)
                .expect("the result node has a table");
            table.update(cx, |table, _| {
                let delegate = table.delegate_mut();
                delegate.adopt(&shared, None, (600.0, 1.0));
                let widened = delegate.adopt(&shared, None, (900.0, 1.0));
                assert!(
                    widened.widths_changed,
                    "a wider body re-resolves the widths"
                );
                assert!(!widened.rows_changed);
            });
        });
    }
}

/// The scroll-absorption rule, which decides whether a wheel belongs to the rows or the canvas.
#[cfg(test)]
mod scroll_tests {
    use gpui_kit::px;

    use super::has_room_to_scroll;

    const MAX: f32 = 1000.0;

    #[test]
    fn a_result_short_enough_to_fit_never_absorbs() {
        assert!(!has_room_to_scroll(px(-120.0), px(0.0), px(0.0)));
        assert!(!has_room_to_scroll(px(120.0), px(0.0), px(0.0)));
    }

    /// Mid-list, both directions belong to the rows.
    #[test]
    fn a_half_scrolled_result_absorbs_either_way() {
        assert!(has_room_to_scroll(px(-120.0), px(-500.0), px(MAX)));
        assert!(has_room_to_scroll(px(120.0), px(-500.0), px(MAX)));
    }

    /// At the top, scrolling up has nowhere to go and the canvas should pan instead — this is
    /// what keeps a table from trapping the gesture.
    #[test]
    fn at_the_top_scrolling_up_falls_through_but_down_absorbs() {
        assert!(!has_room_to_scroll(px(120.0), px(0.0), px(MAX)));
        assert!(has_room_to_scroll(px(-120.0), px(0.0), px(MAX)));
    }

    #[test]
    fn at_the_bottom_scrolling_down_falls_through_but_up_absorbs() {
        assert!(!has_room_to_scroll(px(-120.0), px(-MAX), px(MAX)));
        assert!(has_room_to_scroll(px(120.0), px(-MAX), px(MAX)));
    }

    /// A fractional offset must still read as "at the edge", or the last pixel of a scroll would
    /// leave the table absorbing forever.
    #[test]
    fn a_hair_from_the_edge_still_counts_as_the_edge() {
        assert!(!has_room_to_scroll(px(120.0), px(-0.2), px(MAX)));
        assert!(!has_room_to_scroll(px(-120.0), px(-999.8), px(MAX)));
    }
}
