//! The `TableDelegate` behind a result node.
//!
//! Owns the rows, the resolved column widths and the scale they are rendered at. `TableState`
//! owns everything else — virtualisation, the visible range, scrolling and selection — which is
//! the split `docs/coding-guides.md` asks for: the table owns navigation, the delegate owns
//! presentation.

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui_kit::base::ElementExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::table::{Column as TableColumn, TableDelegate, TableState};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Div, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Stateful,
    Window, div, px,
};
use gpui_kit::{Entity, Focusable, WeakEntity};
use peek_document::ResultSet;
use peek_theme::ActivePeekTheme;

use super::cells;
use super::column_roles::{ColumnRole, Role};
use super::menu::scope::MenuTarget;
use super::outline::{self, Outline};
use super::search::Matches;
use super::selection::{CellRect, Selection};
use super::widths::ColumnWidths;

/// World-unit row height. `ROW_HEIGHT` in `ResultTable.tsx`, where it is only the virtualiser's
/// estimate because rows there are measured; here it is the height, because `uniform_list`
/// requires every row to be identical.
pub(super) const ROW_HEIGHT: f64 = 34.0;

/// World-unit cell padding, from `Result.css`: `7px 14px` on a `td`, `9px 14px` on a `th`.
///
/// gpui-component pads the cell container itself, which would leave the gutters between two
/// selected cells unpainted and break the outline into segments. The columns ask for no padding
/// at all (`Column::p_0`) and the delegate applies it inside the element it owns instead — in
/// world units, so unlike the crate's fixed pixels it scales with the camera the way the row
/// height already does.
const CELL_PADDING: (f64, f64) = (7.0, 14.0);
const HEADER_PADDING: (f64, f64) = (9.0, 14.0);

pub(crate) struct ResultDelegate {
    /// Shared with the sidecar rather than copied: the canvas hands this over on every frame,
    /// so owning a copy meant cloning every cell of every visible result, every frame.
    rows: Arc<ResultSet>,
    widths: ColumnWidths,
    /// The widths the document asked for, kept so a frame can tell a real change from a repeat.
    explicit: Option<BTreeMap<String, f64>>,
    /// World units the body has to spend, so narrow columns can be stretched to fill it.
    available: f64,
    /// Pixels per world unit. Columns are sized in pixels, which do not scale with the rem
    /// scope the node is laid out in, so the delegate converts them itself.
    scale: f64,
    /// What the user has picked out. `DataTable`'s own selection is a single
    /// `Option<(row, col)>`, so the rectangle, the row bands and their mutual exclusivity are
    /// ours; its built-in selection is switched off rather than left to fight this one.
    selection: Selection,
    /// Which rows are on screen and in what order. Searching filters and re-sorts this, so every
    /// row index the table hands us is a **display position** and has to be read through here.
    matches: Matches,
    /// What each column is — a key, a reference, or neither — and what it points at. Recomputed
    /// only when the columns or the schema change; it never moves on its own.
    roles: Vec<ColumnRole>,
    /// The record view: rows become fields and each visible row becomes a column. Held here
    /// rather than read from the node's data on every call because every index in the
    /// `TableDelegate` impl means something different depending on it.
    pivoted: bool,
    /// What a press where the pointer is would select, drawn as a dashed preview
    /// (`useGhostSelection.ts`). `None` while the preview is suppressed, or once the pointer has
    /// left the table.
    ghost: Option<CellRect>,
    /// Where the right-click that opened the context menu landed, cleared when it closes.
    /// The menu acts on this rather than on the selection, which a right press never touches.
    menu_target: Option<MenuTarget>,
    /// The cell or header the pointer is over, kept so the body's right-press catcher knows what
    /// was clicked without repeating the table's column and row arithmetic.
    hovered: Option<MenuTarget>,
    /// The cell whose full value the detail pane is showing.
    detail: Option<(usize, usize)>,
    /// The cell being edited, by **data** row and column, with whatever the last commit said.
    editing: Option<Editing>,
    /// The view that owns this delegate, so a cell's own handlers can reach the commit path.
    /// Weak, because the table is inside it: an `Entity` would be a cycle.
    owner: Option<WeakEntity<super::ResultTable>>,
    /// The field the in-cell editor renders. One per table, reused for whichever cell is open:
    /// `render_td` may not create entities, so it has to exist before editing starts.
    input: Entity<InputState>,
}

/// What one [`ResultDelegate::adopt`] actually changed.
///
/// Three answers rather than one boolean because they have different consequences: only new
/// rows can restale the column roles, only new widths need re-resolving, and a pure zoom needs
/// nothing but the pixel conversion the table caches in its column groups.
#[derive(Debug, Clone, Copy)]
pub(super) struct Adopted {
    pub(super) rows_changed: bool,
    pub(super) widths_changed: bool,
    pub(super) scale_changed: bool,
}

impl Adopted {
    /// Whether `TableState`'s cached column groups still hold the right pixel widths.
    pub(super) fn needs_refresh(self) -> bool {
        self.widths_changed || self.scale_changed
    }
}

/// A cell open for editing.
#[derive(Debug, Clone)]
pub(super) struct Editing {
    pub(super) row: usize,
    pub(super) column: usize,
    /// Why the last commit failed, shown under the cell until the next attempt.
    pub(super) error: Option<String>,
    /// True while the statement is in flight; the editor is read-only meanwhile.
    pub(super) saving: bool,
    /// Whether the JSON popover holds the draft rather than the in-cell field. Two editors
    /// exist and never at once, and the commit path has to read the right one.
    pub(super) popover: bool,
}

impl std::fmt::Debug for ResultDelegate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResultDelegate")
            .field("rows", &self.rows.row_count())
            .field("columns", &self.rows.column_count())
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl ResultDelegate {
    pub(super) fn new(
        rows: Arc<ResultSet>,
        explicit: Option<&BTreeMap<String, f64>>,
        input: Entity<InputState>,
    ) -> Self {
        let widths = ColumnWidths::resolve(&rows, explicit, 0.0);
        let row_count = rows.row_count();
        Self {
            rows,
            widths,
            explicit: explicit.cloned(),
            available: 0.0,
            scale: 1.0,
            selection: Selection::default(),
            matches: Matches::unfiltered(row_count),
            roles: Vec::new(),
            pivoted: false,
            ghost: None,
            hovered: None,
            menu_target: None,
            detail: None,
            editing: None,
            input,
            owner: None,
        }
    }

    /// Set once the owning view exists; the delegate is built first, on its way into it.
    pub(super) fn set_owner(&mut self, owner: WeakEntity<super::ResultTable>) {
        self.owner = Some(owner);
    }

    pub(super) fn owner(&self) -> Option<Entity<super::ResultTable>> {
        self.owner.as_ref()?.upgrade()
    }

    pub(super) fn editing(&self) -> Option<&Editing> {
        self.editing.as_ref()
    }

    pub(super) fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// What a double-click does to a cell.
    ///
    /// A short value edits in place. A JSON value opens the popover editor, which is the only
    /// surface either readable or writable enough for a document — and falls back to the value
    /// pane when the result cannot be written to, since there would be nothing to save. A long
    /// string opens in the pane, which is the only place it fits whole.
    fn open_cell(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let Some(cell) = self.rows.cell(row, column) else {
            return;
        };
        let json = matches!(cell, peek_document::Cell::Json(_));
        let long = cell.to_display_string().chars().count() > super::detail::INLINE_LIMIT;
        let Some(owner) = self.owner() else {
            return;
        };
        if json {
            // Reading the owner back mid-update is the re-entrant borrow documented below, so
            // even the writability question waits for the next turn.
            window.defer(cx, move |window, cx| {
                owner.update(cx, |owner, cx| {
                    owner.open_json_cell(row, column, window, cx);
                });
            });
            return;
        }
        if long {
            self.detail = Some((row, column));
            return;
        }
        // `cx` is mid-update on this table, and opening an editor reads it back — for the
        // editable-table check, the draft, and the input. Doing that here is a re-entrant borrow
        // and aborts the process, which is not a panic a user can recover from. Hand it to the
        // next turn instead, as the Text node does for its own auto-grow.
        window.defer(cx, move |window, cx| {
            owner.update(cx, |owner, cx| owner.begin_edit(row, column, window, cx));
        });
    }

    /// The in-cell editor, when this is the cell being edited. One field is reused for whichever
    /// cell is open, because `render_td` may not create entities.
    fn open_editor(&self, row: usize, column: usize) -> Option<Input> {
        let editing = self
            .editing
            .as_ref()
            // The popover holds the draft for a JSON cell, so the cell itself keeps rendering
            // its preview underneath rather than swapping in a field nothing types into.
            .filter(|edit| edit.row == row && edit.column == column && !edit.popover)?;
        Some(Input::new(&self.input).xsmall().disabled(editing.saving))
    }

    /// Makes the cell report where it ended up, when the JSON popover is anchored to it.
    ///
    /// The panel is chrome the canvas draws, so it has to be told — every frame, which is how it
    /// follows a pan, a zoom or a scroll of the rows.
    fn anchor_json_editor(&self, cell: Stateful<Div>, at: (usize, usize)) -> Stateful<Div> {
        let (row, column) = at;
        let anchored = self
            .editing
            .as_ref()
            .is_some_and(|edit| edit.popover && edit.row == row && edit.column == column);
        let Some(owner) = self.owner().filter(|_| anchored) else {
            return cell;
        };
        cell.on_prepaint(move |bounds, _, cx| {
            owner.update(cx, |table, cx| table.report_json_anchor(bounds, cx));
        })
    }

    pub(super) fn begin_json_edit(&mut self, row: usize, column: usize) {
        self.editing = Some(Editing {
            popover: true,
            row,
            column,
            error: None,
            saving: false,
        });
    }

    pub(super) fn begin_edit(&mut self, row: usize, column: usize) {
        self.editing = Some(Editing {
            popover: false,
            row,
            column,
            error: None,
            saving: false,
        });
    }

    pub(super) fn end_edit(&mut self) {
        self.editing = None;
    }

    pub(super) fn set_edit_state(&mut self, saving: bool, error: Option<String>) {
        if let Some(editing) = self.editing.as_mut() {
            editing.saving = saving;
            editing.error = error;
        }
    }

    pub(super) fn set_roles(&mut self, roles: Vec<ColumnRole>) {
        self.roles = roles;
    }

    pub(super) fn role(&self, column: usize) -> Role {
        self.roles.get(column).map_or(Role::Plain, |role| role.role)
    }

    /// Whether a column's cells are links, for tests: `render_td` builds them, and a test
    /// cannot reach inside a cell that registers no id of its own.
    #[cfg(test)]
    pub(crate) fn follows_references(&self, column: usize) -> bool {
        self.column_role(column)
            .is_some_and(|role| !super::follow::targets(role).is_empty())
    }

    fn column_role(&self, column: usize) -> Option<&ColumnRole> {
        self.roles.get(column)
    }

    /// Asks the database what a reference points at, and puts the answer on the canvas.
    ///
    /// Deferred for the same reason opening an editor is: this runs inside a `TableState`
    /// update, and the owner reads that state back.
    fn follow_reference(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let Some(role) = self.column_role(column) else {
            return;
        };
        let Some(value) = self.rows.cell(row, column) else {
            return;
        };
        let sql_type = self
            .rows
            .columns()
            .get(column)
            .map_or("", |column| column.sql_type.as_str());
        let queries =
            super::follow::queries(role, value, sql_type, crate::database::Database::engine(cx));
        if queries.is_empty() {
            return;
        }
        let Some(owner) = self.owner() else {
            return;
        };
        window.defer(cx, move |_, cx| {
            owner.update(cx, |owner, cx| owner.follow_reference(queries, cx));
        });
    }

    pub(super) fn is_pivoted(&self) -> bool {
        self.pivoted
    }

    /// Adopts the node's pivot flag, dropping everything whose coordinates the transposition
    /// invalidates. Returns whether it changed, because the table has to re-read its columns.
    pub(super) fn set_pivoted(&mut self, pivoted: bool) -> bool {
        if self.pivoted == pivoted {
            return false;
        }
        self.pivoted = pivoted;
        self.selection.clear();
        self.detail = None;
        self.editing = None;
        true
    }

    /// The cell the detail pane is showing, as a **data** row and its column.
    pub(super) fn detail(&self) -> Option<(usize, usize)> {
        self.detail
    }

    pub(super) fn set_detail(&mut self, cell: Option<(usize, usize)>) {
        self.detail = cell;
    }

    pub(super) fn result_rows(&self) -> &ResultSet {
        &self.rows
    }

    pub(crate) fn matches(&self) -> &Matches {
        &self.matches
    }

    /// Adopts a search result, dropping the selection with it: the positions it holds were
    /// captured against the previous ordering.
    pub(super) fn set_matches(&mut self, matches: Matches) {
        self.matches = matches;
        self.selection.clear();
        self.ghost = None;
    }

    /// The data row behind a display position.
    fn row_of(&self, position: usize) -> Option<usize> {
        self.matches.row_at(position)
    }

    /// How the selection describes itself, for the table's accessible label.
    pub(super) fn selection_summary(&self) -> Option<String> {
        super::selection::describe(&self.selection)
    }

    pub(super) fn selection(&self) -> &Selection {
        &self.selection
    }

    pub(super) fn selection_mut(&mut self) -> &mut Selection {
        &mut self.selection
    }

    /// Adopts new rows, or a new width to fill, or a new zoom.
    ///
    /// Returns whether anything changed, because the caller has to `TableState::refresh` when it
    /// did — `column()` is only read on prepare and refresh, so a width change is invisible
    /// until then.
    pub(super) fn adopt(
        &mut self,
        rows: &Arc<ResultSet>,
        explicit: Option<&BTreeMap<String, f64>>,
        layout: (f64, f64),
    ) -> Adopted {
        let (available, scale) = layout;
        // The canvas hands over the same `Arc` on every frame, so the pointer answers "nothing
        // has run" for free — which is the whole point of sharing the set rather than copying
        // it. Only when the pointer moves has a query actually finished, and only then is a
        // comparison by value worth its walk over every cell: a **live** query re-runs every ten
        // seconds and usually gets identical rows back, and dropping the user's selection,
        // search and open cell on each of those would make a live result unusable.
        let same_set = Arc::ptr_eq(&self.rows, rows);
        let rows_changed = !same_set && self.rows != *rows;
        let widths_changed = (self.available - available).abs() > 0.5
            || self.explicit.as_ref() != explicit
            || rows_changed;
        let scale_changed = (self.scale - scale).abs() > f64::EPSILON;

        // Adopted whatever the values said, so the next frame gets its answer from the pointer
        // again rather than re-walking a set it has already compared once.
        if !same_set {
            self.rows = Arc::clone(rows);
        }
        // The scale is only ever a multiplier on the way to pixels (see `pixels`), so adopting
        // it can never invalidate the widths, which are world units.
        self.scale = scale;
        if !widths_changed {
            return Adopted {
                rows_changed: false,
                widths_changed: false,
                scale_changed,
            };
        }
        if rows_changed {
            // A position only means something against the ordering it was captured in, so new
            // rows drop both the selection and any search.
            self.selection.clear();
            self.ghost = None;
            self.matches = Matches::unfiltered(self.rows.row_count());
            self.detail = None;
            self.editing = None;
        }
        self.available = available;
        self.explicit = explicit.cloned();
        self.widths = ColumnWidths::resolve(&self.rows, explicit, available);
        Adopted {
            rows_changed,
            widths_changed: true,
            scale_changed,
        }
    }

    pub(super) fn scale(&self) -> f64 {
        self.scale
    }

    /// How many rows the table is showing. Not the result's row count: a search filters the
    /// display, and a column selection has to stop at the last row actually on screen.
    pub(super) fn visible_rows(&self) -> usize {
        self.matches.len()
    }

    pub(super) fn column_names(&self) -> impl Iterator<Item = &str> {
        self.rows
            .columns()
            .iter()
            .map(|column| column.name.as_str())
    }

    fn pixels(&self, world: f64) -> Pixels {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a column width in pixels is far inside f32"
        )]
        px((world * self.scale) as f32)
    }

    /// The dashed preview of what pressing here would select, or `None` when there is nothing to
    /// preview. `useGhostSelection.ts`: a header ghosts its whole column, shift ghosts the whole
    /// row, and a plain hover ghosts the one cell — but never while a button is down, never while
    /// cmd/ctrl turns the drag into a node move, and never over a cell already selected, where
    /// the preview would only blur the selection it sits on.
    fn ghost_for(&self, at: (usize, usize), modifiers: gpui_kit::Modifiers) -> Option<CellRect> {
        if modifiers.secondary() {
            return None;
        }
        let (row, column) = at;
        let rect = if modifiers.shift {
            CellRect::row(row, self.columns_count_now())
        } else {
            CellRect::cell(row, column)
        };
        (!self.selection.contains_cell(row, column)).then_some(rect)
    }

    /// The whole column, as hovering or pressing a header takes it.
    fn ghost_column(&self, column: usize, modifiers: gpui_kit::Modifiers) -> Option<CellRect> {
        (!modifiers.secondary()).then(|| CellRect::column(column, self.matches.len()))
    }

    pub(super) fn menu_target(&self) -> Option<MenuTarget> {
        self.menu_target
    }

    /// What the pointer is over, which a right press turns into a menu target.
    pub(super) fn hovered(&self) -> Option<MenuTarget> {
        self.hovered
    }

    fn set_hovered(&mut self, hovered: Option<MenuTarget>) {
        self.hovered = hovered;
    }

    pub(super) fn set_menu_target(&mut self, target: Option<MenuTarget>) {
        self.menu_target = target;
    }

    /// Drops the preview outright, for a caller that only knows the pointer has gone.
    pub(super) fn clear_ghost(&mut self) -> bool {
        self.hovered = None;
        self.set_ghost(None)
    }

    /// Adopts a ghost only when it actually moved: a mouse move fires many times a second, and
    /// notifying on each one would repaint the whole node for nothing.
    fn set_ghost(&mut self, ghost: Option<CellRect>) -> bool {
        let changed = self.ghost != ghost;
        self.ghost = ghost;
        changed
    }

    /// How many columns the table is showing right now, which the record view transposes.
    fn columns_count_now(&self) -> usize {
        if self.pivoted {
            1 + self.matches.len()
        } else {
            self.rows.column_count()
        }
    }

    /// The two outlines a cell may carry: the live selection, and under it the dashed preview.
    /// Ordered so a solid edge wins wherever they overlap, which is what the `::after` over
    /// `::before` stacking buys in the stylesheet.
    fn outlines(&self, row: usize, column: usize, cx: &App) -> Vec<gpui_kit::AnyElement> {
        let columns = self.columns_count_now();
        let ghost = self
            .ghost
            .map(|rect| rect.edges(row, column))
            .unwrap_or_default();
        [
            outline::overlay(Outline::Ghost, ghost, cx),
            outline::overlay(
                Outline::Selected,
                self.selection.edges(row, column, columns),
                cx,
            ),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// A cell whose column really references another table is a link: pressing it claims the
    /// press so the table does not also start a selection, which is the rule `useCellSelection`
    /// follows for `.reference--link`.
    fn link_cell(
        &self,
        at: (usize, usize),
        value: (usize, &peek_document::Cell),
        cx: &mut Context<TableState<Self>>,
    ) -> gpui_kit::AnyElement {
        let (row_ix, col_ix) = at;
        let (row, value) = value;
        div()
            .id(("result-link", row_ix * 1000 + col_ix))
            // Fills the cell, so the whole value is the target rather than whatever width the
            // text happens to take.
            .size_full()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()),
            )
            .on_click(cx.listener(move |table, _, window, cx| {
                table
                    .delegate_mut()
                    .follow_reference(row, col_ix, window, cx);
            }))
            .child(cells::cell(value, self.role(col_ix), cx))
            .into_any_element()
    }
}

impl TableDelegate for ResultDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        if self.pivoted {
            // One field column, then one column per record the search left visible.
            return 1 + self.matches.len();
        }
        self.rows.column_count()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        if self.pivoted {
            return self.rows.column_count();
        }
        self.matches.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> TableColumn {
        if self.pivoted {
            return self.pivot_column(col_ix);
        }
        let Some(column) = self.rows.columns().get(col_ix) else {
            return TableColumn::new("", "");
        };
        TableColumn::new(column.name.clone(), column.name.clone())
            .width(self.pixels(self.widths.get(col_ix)))
            // Dragging a column edge is the one table gesture that works before the canvas
            // learns to hand a node its own presses, because the table owns those hitboxes.
            .resizable(true)
            // The reference has no sorting or reordering: clicking a header selects the column.
            .movable(false)
            // The delegate pads its own element instead (see `CELL_PADDING`), so a cell's
            // background and its selection outline reach the column's real edges and a press
            // anywhere in the header — padding included — lands on a handler that wants it.
            .p_0()
            .min_width(px(40.0))
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if self.pivoted {
            return self.pivot_th(col_ix, cx);
        }
        let Some(column) = self.rows.columns().get(col_ix) else {
            return div().into_any_element();
        };
        // The reference gives a `th` no selected state: only the cells under it change, so the
        // header never disagrees with the outline drawn around the column.
        let role = self.role(col_ix);
        let (vertical, horizontal) = HEADER_PADDING;

        div()
            .id(("result-th", col_ix))
            .size_full()
            .py(self.pixels(vertical))
            .px(self.pixels(horizontal))
            // `thead th { cursor: pointer }`: a header selects its column, and nothing else in
            // the table says so. The tint lands on the name rather than the cell, so the group.
            .cursor_pointer()
            .group(cells::HEADER_GROUP)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                    if event.modifiers.secondary() {
                        return;
                    }
                    // The node body clears the selection on any press that is not on a cell, so
                    // without claiming this one the column would be selected and wiped by the
                    // same event — which is exactly how headers came to look unclickable.
                    cx.stop_propagation();
                    // And pressing a header focuses the table for the same reason pressing a
                    // cell does: `escape` and `cmd-c` dispatch outward from whatever has focus.
                    window.focus(&table.focus_handle(cx), cx);
                    let rows = table.delegate().visible_rows();
                    table
                        .delegate_mut()
                        .selection_mut()
                        .press_header(col_ix, rows);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |table, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    table.delegate_mut().set_hovered(Some(MenuTarget {
                        row: 0,
                        position: 0,
                        column: col_ix,
                        header: true,
                    }));
                    let ghost = table.delegate().ghost_column(col_ix, event.modifiers);
                    if table.delegate_mut().set_ghost(ghost) {
                        cx.notify();
                    }
                    return;
                }
                let rows = table.delegate().visible_rows();
                table
                    .delegate_mut()
                    .selection_mut()
                    .drag_to_header(col_ix, rows);
                cx.notify();
            }))
            .child(cells::header(&column.name, &column.sql_type, role, cx))
            .into_any_element()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if self.pivoted {
            return self.pivot_td(row_ix, col_ix, cx);
        }
        let Some(row) = self.row_of(row_ix) else {
            return div().into_any_element();
        };
        // A refcount bump, not a copy of the rows: cloning the `Cell` here deep-cloned the whole
        // `serde_json::Value` of every visible JSON cell, every frame. Borrowing through a local
        // handle keeps the value out of `self`'s borrow for the rest of the method.
        let rows = Arc::clone(&self.rows);
        let Some(value) = rows.cell(row, col_ix) else {
            return div().into_any_element();
        };
        let matched = self.matches.cell(row, col_ix).is_some();

        let in_rect = self.selection.contains_cell(row_ix, col_ix);
        let in_row = self.selection.is_row_selected(row_ix);
        let theme = cx.peek_theme();
        let (vertical, horizontal) = CELL_PADDING;

        let mut cell = div()
            .id(("result-td", row_ix * 1000 + col_ix))
            .size_full()
            // The outlines below are absolute children, so the cell has to be their frame.
            .relative()
            .py(self.pixels(vertical))
            .px(self.pixels(horizontal));
        if in_rect || in_row {
            cell = cell.bg(theme.row_selected_bg);
        } else if matched {
            // `.search-match` in the stylesheet: a matched cell is tinted so a hit is findable
            // by eye without reading every row.
            cell = cell.bg(theme.node_bg_2);
        }

        let links = self
            .column_role(col_ix)
            .is_some_and(|role| !super::follow::targets(role).is_empty())
            && !value.is_null();

        if let Some(open) = self.open_editor(row, col_ix) {
            return cell
                .border_1()
                .border_color(theme.accent)
                .child(open)
                .into_any_element();
        }

        let outlines = self.outlines(row_ix, col_ix, cx);

        cell = self.anchor_json_editor(cell, (row, col_ix));

        cell.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                if event.modifiers.secondary() {
                    return;
                }
                cx.stop_propagation();
                // Pressing into the table gives it focus, so `escape` and `cmd-c` dispatch
                // through it and reach the node's own handlers. `CanvasView::reclaim_focus`
                // covers the handle dying with the node.
                window.focus(&table.focus_handle(cx), cx);
                table.delegate_mut().selection_mut().press_cell(
                    row_ix,
                    col_ix,
                    event.modifiers.shift,
                );
                // The preview is of a press that has now happened, and it would otherwise sit
                // dashed over the selection it just became.
                table.delegate_mut().set_ghost(None);
                cx.notify();
            }),
        )
        .on_mouse_move(cx.listener(move |table, event: &MouseMoveEvent, _, cx| {
            if event.pressed_button != Some(MouseButton::Left) {
                table.delegate_mut().set_hovered(Some(MenuTarget {
                    row,
                    position: row_ix,
                    column: col_ix,
                    header: false,
                }));
                let ghost = table
                    .delegate()
                    .ghost_for((row_ix, col_ix), event.modifiers);
                if table.delegate_mut().set_ghost(ghost) {
                    cx.notify();
                }
                return;
            }
            table
                .delegate_mut()
                .selection_mut()
                .drag_to_cell(row_ix, col_ix);
            cx.notify();
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |table, event: &MouseUpEvent, window, cx| {
                table.delegate_mut().selection_mut().release();
                // A double-click opens the cell's full value, which is the only way to read a
                // JSON object or a long string: the grid shows a one-line summary of both.
                if event.click_count >= 2 {
                    table.delegate_mut().open_cell(row, col_ix, window, cx);
                }
                cx.notify();
            }),
        )
        .children(outlines)
        .child(if links {
            self.link_cell((row_ix, col_ix), (row, value), cx)
        } else {
            cells::cell(value, self.role(col_ix), cx)
        })
        .into_any_element()
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        // "No matching rows" distinguishes a search that found nothing from a query that
        // returned nothing, which is the difference the user needs to see.
        let message = if self.matches.is_searching() {
            "No matching rows"
        } else {
            "No results"
        };
        super::empty_state(message, cx)
    }

    /// The table's own CSV/copy path reads cells through this, so it has to agree with what the
    /// cell shows: `stringifyValue`, where NULL is the empty string.
    fn cell_text(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        if self.pivoted {
            return self.pivot_cell_text(row_ix, col_ix);
        }
        self.row_of(row_ix)
            .and_then(|row| self.rows.cell(row, col_ix))
            .map(peek_document::Cell::to_display_string)
            .unwrap_or_default()
    }
}

/// The record view: `ResultPivotView.tsx`.
///
/// Every index flips. A table row is one of the result's *columns*, and a table column is one
/// *record* — the first holding the field names. The reference stacks each record as its own
/// vertical table; `DataTable` has one grid and virtualises it, so the records sit side by side
/// instead, which is the same information in the shape this table can draw.
///
/// It is a read view: the cell selection, column resizing and inline editing all address the
/// untransposed grid, and no coordinate in them would survive the flip.
mod pivot {
    /// World units. The field column carries a name and a type, so it is the wider of the two.
    pub(super) const FIELD_WIDTH: f64 = 190.0;
    pub(super) const VALUE_WIDTH: f64 = 240.0;
}

impl ResultDelegate {
    fn pivot_column(&self, col_ix: usize) -> TableColumn {
        if col_ix == 0 {
            return TableColumn::new("pivot-field", "Field")
                .width(self.pixels(pivot::FIELD_WIDTH))
                .movable(false)
                .p_0()
                // A width dragged here would be written back under a real column's name, which
                // is a width the table view would then wear.
                .resizable(false);
        }
        let name = gpui_kit::SharedString::from(format!("#{col_ix}"));
        TableColumn::new(name.clone(), name)
            .width(self.pixels(pivot::VALUE_WIDTH))
            .movable(false)
            .p_0()
            .resizable(false)
    }

    fn pivot_th(&self, col_ix: usize, cx: &App) -> gpui_kit::AnyElement {
        let theme = cx.peek_theme();
        let label = if col_ix == 0 {
            "Field".to_string()
        } else {
            format!("#{col_ix}")
        };
        let (vertical, horizontal) = HEADER_PADDING;
        div()
            .size_full()
            .flex()
            .items_center()
            .py(self.pixels(vertical))
            .px(self.pixels(horizontal))
            .text_color(theme.fg_subtle)
            .child(label)
            .into_any_element()
    }

    fn pivot_td(
        &self,
        row_ix: usize,
        col_ix: usize,
        cx: &mut Context<TableState<Self>>,
    ) -> gpui_kit::AnyElement {
        let Some(column) = self.rows.columns().get(row_ix) else {
            return div().into_any_element();
        };
        let (name, sql_type) = (column.name.clone(), column.sql_type.clone());
        let role = self.role(row_ix);
        let field_bg = cx.peek_theme().node_bg_2;
        let (vertical, horizontal) = CELL_PADDING;

        if col_ix == 0 {
            return div()
                .size_full()
                .bg(field_bg)
                .py(self.pixels(vertical))
                .px(self.pixels(horizontal))
                .child(super::cells::header(&name, &sql_type, role, cx))
                .into_any_element();
        }

        let Some(row) = self.row_of(col_ix - 1) else {
            return div().into_any_element();
        };
        // As in `render_td`: a handle, so a JSON value is not deep-cloned once a frame.
        let rows = Arc::clone(&self.rows);
        let Some(value) = rows.cell(row, row_ix) else {
            return div().into_any_element();
        };
        let matched = self.matches.cell(row, row_ix).is_some();
        let match_bg = cx.peek_theme().node_bg_2;

        div()
            .id(("result-pivot-td", row_ix * 1000 + col_ix))
            .size_full()
            .py(self.pixels(vertical))
            .px(self.pixels(horizontal))
            .when(matched, |cell| cell.bg(match_bg))
            // A long value or a JSON object is a one-line summary here as it is in the table, so
            // the pane is the only way to read it whole.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |table, event: &MouseUpEvent, _, cx| {
                    if event.click_count >= 2 {
                        table.delegate_mut().set_detail(Some((row, row_ix)));
                        cx.notify();
                    }
                }),
            )
            .child(super::cells::cell(value, role, cx))
            .into_any_element()
    }

    fn pivot_cell_text(&self, row_ix: usize, col_ix: usize) -> String {
        let Some(column) = self.rows.columns().get(row_ix) else {
            return String::new();
        };
        if col_ix == 0 {
            return column.name.clone();
        }
        self.row_of(col_ix - 1)
            .and_then(|row| self.rows.cell(row, row_ix))
            .map(peek_document::Cell::to_display_string)
            .unwrap_or_default()
    }
}
