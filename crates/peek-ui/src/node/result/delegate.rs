//! The `TableDelegate` behind a result node.
//!
//! Owns the rows, the resolved column widths and the scale they are rendered at. `TableState`
//! owns everything else — virtualisation, the visible range, scrolling and selection — which is
//! the split `docs/coding-guides.md` asks for: the table owns navigation, the delegate owns
//! presentation.

use std::collections::BTreeMap;

use gpui_kit::Focusable;
use gpui_kit::component::table::{Column as TableColumn, TableDelegate, TableState};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Window, div,
    px,
};
use peek_document::ResultSet;
use peek_theme::ActivePeekTheme;

use super::cells;
use super::column_roles::{ColumnRole, Role};
use super::search::Matches;
use super::selection::Selection;
use super::widths::ColumnWidths;

/// World-unit row height. `ROW_HEIGHT` in `ResultTable.tsx`, where it is only the virtualiser's
/// estimate because rows there are measured; here it is the height, because `uniform_list`
/// requires every row to be identical.
pub(super) const ROW_HEIGHT: f64 = 34.0;

pub(crate) struct ResultDelegate {
    rows: ResultSet,
    widths: ColumnWidths,
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
    /// The cell whose full value the detail pane is showing.
    detail: Option<(usize, usize)>,
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
    pub(super) fn new(rows: ResultSet, explicit: Option<&BTreeMap<String, f64>>) -> Self {
        let widths = ColumnWidths::resolve(&rows, explicit, 0.0);
        let row_count = rows.row_count();
        Self {
            rows,
            widths,
            available: 0.0,
            scale: 1.0,
            selection: Selection::default(),
            matches: Matches::unfiltered(row_count),
            roles: Vec::new(),
            detail: None,
        }
    }

    pub(super) fn set_roles(&mut self, roles: Vec<ColumnRole>) {
        self.roles = roles;
    }

    pub(super) fn role(&self, column: usize) -> Role {
        self.roles.get(column).map_or(Role::Plain, |role| role.role)
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
        rows: &ResultSet,
        explicit: Option<&BTreeMap<String, f64>>,
        layout: (f64, f64),
    ) -> bool {
        let (available, scale) = layout;
        let rows_changed = &self.rows != rows;
        let layout_changed =
            (self.available - available).abs() > 0.5 || (self.scale - scale).abs() > f64::EPSILON;
        if !rows_changed && !layout_changed {
            return false;
        }
        if rows_changed {
            self.rows = rows.clone();
            // A position only means something against the ordering it was captured in, so new
            // rows drop both the selection and any search.
            self.selection.clear();
            self.matches = Matches::unfiltered(self.rows.row_count());
            self.detail = None;
        }
        self.available = available;
        self.scale = scale;
        self.widths = ColumnWidths::resolve(&self.rows, explicit, available);
        true
    }

    pub(super) fn scale(&self) -> f64 {
        self.scale
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
}

impl TableDelegate for ResultDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.rows.column_count()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.matches.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> TableColumn {
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
            .min_width(px(40.0))
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(column) = self.rows.columns().get(col_ix) else {
            return div().into_any_element();
        };
        let selected = self
            .selection
            .rect()
            .is_some_and(|rect| rect.columns().contains(&col_ix));
        let theme = cx.peek_theme();
        let background = if selected {
            theme.row_selected_bg
        } else {
            theme.node_bg
        };
        let rows = self.rows.row_count();
        let role = self.role(col_ix);

        div()
            .id(("result-th", col_ix))
            .size_full()
            .bg(background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, event: &MouseDownEvent, _, cx| {
                    if event.modifiers.secondary() {
                        return;
                    }
                    table
                        .delegate_mut()
                        .selection_mut()
                        .press_header(col_ix, rows);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |table, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
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
        let Some(row) = self.row_of(row_ix) else {
            return div().into_any_element();
        };
        let Some(value) = self.rows.cell(row, col_ix).cloned() else {
            return div().into_any_element();
        };
        let matched = self.matches.cell(row, col_ix).is_some();
        let sql_type = self
            .rows
            .columns()
            .get(col_ix)
            .map_or(String::new(), |column| column.sql_type.clone());

        let in_rect = self.selection.contains_cell(row_ix, col_ix);
        let in_row = self.selection.is_row_selected(row_ix);
        let (band_top, band_bottom) = self.selection.band_edges(row_ix);
        let theme = cx.peek_theme();

        let mut cell = div().id(("result-td", row_ix * 1000 + col_ix)).size_full();
        if in_rect || in_row {
            cell = cell.bg(theme.row_selected_bg);
        } else if matched {
            // `.search-match` in the stylesheet: a matched cell is tinted so a hit is findable
            // by eye without reading every row.
            cell = cell.bg(theme.node_bg_2);
        }
        // A run of selected rows is outlined as one band, not one box per row.
        if in_row && band_top {
            cell = cell.border_t_1().border_color(theme.accent);
        }
        if in_row && band_bottom {
            cell = cell.border_b_1().border_color(theme.accent);
        }

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
                cx.notify();
            }),
        )
        .on_mouse_move(cx.listener(move |table, event: &MouseMoveEvent, _, cx| {
            if event.pressed_button != Some(MouseButton::Left) {
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
            cx.listener(move |table, event: &MouseUpEvent, _, cx| {
                table.delegate_mut().selection_mut().release();
                // A double-click opens the cell's full value, which is the only way to read a
                // JSON object or a long string: the grid shows a one-line summary of both.
                if event.click_count >= 2 {
                    table.delegate_mut().set_detail(Some((row, col_ix)));
                }
                cx.notify();
            }),
        )
        .child(cells::cell(&value, &sql_type, self.role(col_ix), cx))
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
        self.row_of(row_ix)
            .and_then(|row| self.rows.cell(row, col_ix))
            .map(peek_document::Cell::to_display_string)
            .unwrap_or_default()
    }
}
