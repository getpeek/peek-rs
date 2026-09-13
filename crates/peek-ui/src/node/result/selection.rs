//! What is selected in a result table, and the gestures that change it.
//!
//! Ported from `useCellSelection.ts` and `useRowSelection.ts`. Pure index arithmetic with no
//! gpui in it, so every rule below is a plain unit test.
//!
//! Two kinds of selection, deliberately **mutually exclusive** (`useResultSelections.ts`):
//! a rectangle of cells, or a set of whole rows. Starting either clears the other, because a
//! copy has to mean one unambiguous thing.
//!
//! Positions here are *display* positions — where a row sits on screen — not indices into the
//! result. They are the same thing until search lands and re-sorts the rows by match score; the
//! reference already separates them, and the names keep that seam visible.

use std::collections::BTreeSet;

use peek_document::ResultSet;

/// An inclusive rectangle of cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CellRect {
    pub(super) top: usize,
    pub(super) bottom: usize,
    pub(super) left: usize,
    pub(super) right: usize,
}

impl CellRect {
    fn between(anchor: (usize, usize), focus: (usize, usize)) -> Self {
        Self {
            top: anchor.0.min(focus.0),
            bottom: anchor.0.max(focus.0),
            left: anchor.1.min(focus.1),
            right: anchor.1.max(focus.1),
        }
    }

    pub(super) fn contains(self, row: usize, column: usize) -> bool {
        (self.top..=self.bottom).contains(&row) && (self.left..=self.right).contains(&column)
    }

    pub(super) fn rows(self) -> std::ops::RangeInclusive<usize> {
        self.top..=self.bottom
    }

    pub(super) fn columns(self) -> std::ops::RangeInclusive<usize> {
        self.left..=self.right
    }

    pub(super) fn area(self) -> usize {
        (self.bottom - self.top + 1) * (self.right - self.left + 1)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Selection {
    cells: Option<Cells>,
    rows: Rows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cells {
    anchor: (usize, usize),
    focus: (usize, usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Rows {
    selected: BTreeSet<usize>,
    /// The row a shift-drag started from, and the selection as it was then, so an in-flight
    /// sweep composes with what was already selected instead of replacing it.
    drag: Option<(usize, BTreeSet<usize>)>,
    /// Whether the sweep has actually moved. A shift-press that never moves is a *toggle*, which
    /// is only known on release.
    moved: bool,
}

impl Selection {
    pub(super) fn is_empty(&self) -> bool {
        self.cells.is_none() && self.rows.selected.is_empty()
    }

    pub(super) fn rect(&self) -> Option<CellRect> {
        self.cells
            .map(|cells| CellRect::between(cells.anchor, cells.focus))
    }

    pub(super) fn selected_rows(&self) -> &BTreeSet<usize> {
        &self.rows.selected
    }

    pub(super) fn is_row_selected(&self, row: usize) -> bool {
        self.rows.selected.contains(&row)
    }

    pub(super) fn contains_cell(&self, row: usize, column: usize) -> bool {
        self.rect().is_some_and(|rect| rect.contains(row, column))
    }

    /// Whether a selected row is at the top or bottom edge of its band, which is what lets the
    /// outline be drawn around a run of rows rather than around each one.
    pub(super) fn band_edges(&self, row: usize) -> (bool, bool) {
        if !self.is_row_selected(row) {
            return (false, false);
        }
        let above = row
            .checked_sub(1)
            .is_none_or(|previous| !self.is_row_selected(previous));
        (above, !self.is_row_selected(row + 1))
    }

    /// Clears everything. Escape does this, and so does any change to the rows, because a
    /// position only means something against the ordering it was captured in.
    pub(super) fn clear(&mut self) -> bool {
        let changed = !self.is_empty();
        *self = Self::default();
        changed
    }

    /// A press on a cell. Shift selects rows, anything else starts a rectangle.
    pub(super) fn press_cell(&mut self, row: usize, column: usize, shift: bool) {
        if shift {
            self.press_row(row);
            return;
        }
        self.rows = Rows::default();
        self.cells = Some(Cells {
            anchor: (row, column),
            focus: (row, column),
        });
    }

    /// A press on a column header selects the whole column; dragging across headers extends the
    /// selection to a range of columns.
    pub(super) fn press_header(&mut self, column: usize, row_count: usize) {
        if row_count == 0 {
            return;
        }
        self.rows = Rows::default();
        self.cells = Some(Cells {
            anchor: (0, column),
            focus: (row_count - 1, column),
        });
    }

    fn press_row(&mut self, row: usize) {
        self.cells = None;
        let baseline = self.rows.selected.clone();
        self.rows.drag = Some((row, baseline));
        self.rows.moved = false;
    }

    /// The pointer moved to another cell with the button still down.
    pub(super) fn drag_to_cell(&mut self, row: usize, column: usize) {
        if let Some((anchor, baseline)) = self.rows.drag.clone() {
            self.rows.moved = true;
            let (low, high) = (anchor.min(row), anchor.max(row));
            self.rows.selected = baseline.into_iter().chain(low..=high).collect();
            return;
        }
        if let Some(cells) = self.cells.as_mut() {
            cells.focus = (row, column);
        }
    }

    /// Dragging across the header extends a column range, keeping every row.
    pub(super) fn drag_to_header(&mut self, column: usize, row_count: usize) {
        if row_count == 0 {
            return;
        }
        if let Some(cells) = self.cells.as_mut() {
            cells.focus = (row_count - 1, column);
        }
    }

    /// The button came up. A shift-press that never moved is a toggle, which is the only thing
    /// that can add a single row to a selection without sweeping through everything between.
    pub(super) fn release(&mut self) {
        let Some((anchor, baseline)) = self.rows.drag.take() else {
            return;
        };
        if self.rows.moved {
            return;
        }
        let mut next = baseline;
        if !next.remove(&anchor) {
            next.insert(anchor);
        }
        self.rows.selected = next;
    }
}

/// The selection as tab-separated text, which pastes cleanly into a spreadsheet.
///
/// `stringifyValue`'s rule throughout: NULL is the empty string, so a blank cell arrives blank
/// rather than spelling out "null".
///
/// `visible` maps the display positions a selection holds onto rows of the result, so copying
/// from a searched table copies what is on screen rather than whatever sits at those indices.
pub(super) fn to_tsv(rows: &ResultSet, selection: &Selection, visible: &[usize]) -> Option<String> {
    let row_at = |position: usize| visible.get(position).copied();
    if let Some(rect) = selection.rect() {
        return Some(join(
            rect.rows()
                .filter_map(row_at)
                .map(|row| join_row(rows, row, rect.columns())),
        ));
    }
    let selected = selection.selected_rows();
    if selected.is_empty() {
        return None;
    }
    let last = rows.column_count().saturating_sub(1);
    Some(join(
        selected
            .iter()
            .filter_map(|position| row_at(*position))
            .map(|row| join_row(rows, row, 0..=last)),
    ))
}

fn join_row(rows: &ResultSet, row: usize, columns: std::ops::RangeInclusive<usize>) -> String {
    columns
        .map(|column| {
            rows.cell(row, column)
                .map(peek_document::Cell::to_display_string)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\t")
}

fn join(lines: impl Iterator<Item = String>) -> String {
    lines.collect::<Vec<_>>().join("\n")
}

/// How the selection describes itself, for the copy affordance and later the toolbar.
pub(super) fn describe(selection: &Selection) -> Option<String> {
    if let Some(rect) = selection.rect() {
        let area = rect.area();
        if area == 1 {
            return Some("1 cell".to_string());
        }
        return Some(format!("{area} cells"));
    }
    match selection.selected_rows().len() {
        0 => None,
        1 => Some("1 row".to_string()),
        many => Some(format!("{many} rows")),
    }
}

#[cfg(test)]
mod tests {
    use peek_document::{Cell, Column, ResultSet};

    use super::{Selection, describe, to_tsv};

    fn rows() -> ResultSet {
        ResultSet::new(
            vec![
                Column::new("id", "INT4"),
                Column::new("name", "VARCHAR"),
                Column::new("note", "TEXT"),
            ],
            (0..4)
                .map(|index| {
                    vec![
                        Cell::Int(index),
                        Cell::Text(format!("name{index}")),
                        if index == 2 {
                            Cell::Null
                        } else {
                            Cell::Text("x".into())
                        },
                    ]
                })
                .collect(),
        )
    }

    #[test]
    fn a_press_selects_one_cell() {
        let mut selection = Selection::default();
        selection.press_cell(1, 2, false);
        let rect = selection.rect().expect("a rect");
        assert_eq!((rect.top, rect.bottom, rect.left, rect.right), (1, 1, 2, 2));
        assert!(selection.contains_cell(1, 2));
        assert!(!selection.contains_cell(1, 1));
    }

    #[test]
    fn dragging_extends_the_rectangle_in_any_direction() {
        let mut selection = Selection::default();
        selection.press_cell(2, 2, false);
        selection.drag_to_cell(0, 0);
        let rect = selection.rect().expect("a rect");
        assert_eq!((rect.top, rect.bottom, rect.left, rect.right), (0, 2, 0, 2));
        assert!(selection.contains_cell(1, 1), "the rect normalises");
    }

    #[test]
    fn a_header_press_takes_the_whole_column() {
        let mut selection = Selection::default();
        selection.press_header(1, 4);
        let rect = selection.rect().expect("a rect");
        assert_eq!((rect.top, rect.bottom, rect.left, rect.right), (0, 3, 1, 1));
    }

    #[test]
    fn dragging_across_headers_extends_to_a_column_range() {
        let mut selection = Selection::default();
        selection.press_header(0, 4);
        selection.drag_to_header(2, 4);
        let rect = selection.rect().expect("a rect");
        assert_eq!((rect.top, rect.bottom, rect.left, rect.right), (0, 3, 0, 2));
    }

    /// A header press on an empty result must not produce a rect over rows that do not exist.
    #[test]
    fn a_header_press_on_an_empty_result_selects_nothing() {
        let mut selection = Selection::default();
        selection.press_header(0, 0);
        assert!(selection.rect().is_none());
    }

    #[test]
    fn shift_press_and_release_toggles_a_row() {
        let mut selection = Selection::default();
        selection.press_cell(1, 0, true);
        selection.release();
        assert!(selection.is_row_selected(1));

        selection.press_cell(1, 0, true);
        selection.release();
        assert!(!selection.is_row_selected(1), "a second toggle removes it");
    }

    #[test]
    fn shift_dragging_sweeps_a_band_and_keeps_what_was_already_selected() {
        let mut selection = Selection::default();
        selection.press_cell(0, 0, true);
        selection.release();

        selection.press_cell(2, 0, true);
        selection.drag_to_cell(3, 0);
        selection.release();

        assert!(
            selection.is_row_selected(0),
            "the baseline survived the sweep"
        );
        assert!(selection.is_row_selected(2));
        assert!(selection.is_row_selected(3));
        assert!(
            !selection.is_row_selected(1),
            "and the gap stayed unselected"
        );
    }

    /// A sweep that moved is a range, not a toggle — otherwise dragging over the row you started
    /// on would deselect it on release.
    #[test]
    fn a_sweep_that_moved_is_not_also_a_toggle() {
        let mut selection = Selection::default();
        selection.press_cell(1, 0, true);
        selection.drag_to_cell(2, 0);
        selection.release();
        assert!(selection.is_row_selected(1));
        assert!(selection.is_row_selected(2));
    }

    /// One copy has to mean one thing, so the two kinds never coexist.
    #[test]
    fn the_two_selections_are_mutually_exclusive() {
        let mut selection = Selection::default();
        selection.press_cell(0, 0, true);
        selection.release();
        assert!(!selection.selected_rows().is_empty());

        selection.press_cell(2, 1, false);
        assert!(
            selection.selected_rows().is_empty(),
            "rows gave way to cells"
        );
        assert!(selection.rect().is_some());

        selection.press_cell(3, 0, true);
        selection.release();
        assert!(selection.rect().is_none(), "and cells gave way to rows");
    }

    #[test]
    fn band_edges_wrap_a_run_of_rows_not_each_one() {
        let mut selection = Selection::default();
        selection.press_cell(1, 0, true);
        selection.drag_to_cell(3, 0);
        selection.release();

        assert_eq!(selection.band_edges(1), (true, false), "top of the band");
        assert_eq!(selection.band_edges(2), (false, false), "inside it");
        assert_eq!(selection.band_edges(3), (false, true), "bottom of the band");
        assert_eq!(
            selection.band_edges(0),
            (false, false),
            "not selected at all"
        );
    }

    /// The first row has nothing above it, so it is always a top edge.
    #[test]
    fn the_first_row_is_a_band_edge() {
        let mut selection = Selection::default();
        selection.press_cell(0, 0, true);
        selection.release();
        assert_eq!(selection.band_edges(0), (true, true));
    }

    #[test]
    fn clearing_reports_whether_anything_was_selected() {
        let mut selection = Selection::default();
        assert!(!selection.clear(), "nothing to clear");
        selection.press_cell(0, 0, false);
        assert!(selection.clear());
        assert!(selection.is_empty());
    }

    #[test]
    fn a_cell_rectangle_copies_as_tab_separated_rows() {
        let mut selection = Selection::default();
        selection.press_cell(0, 0, false);
        selection.drag_to_cell(1, 1);
        assert_eq!(
            to_tsv(&rows(), &selection, &[0, 1, 2, 3]).as_deref(),
            Some("0\tname0\n1\tname1")
        );
    }

    /// NULL copies blank, so a spreadsheet gets an empty cell rather than the word.
    #[test]
    fn a_null_copies_as_an_empty_field() {
        let mut selection = Selection::default();
        selection.press_cell(2, 2, false);
        assert_eq!(
            to_tsv(&rows(), &selection, &[0, 1, 2, 3]).as_deref(),
            Some("")
        );
    }

    #[test]
    fn selected_rows_copy_whole_and_in_order() {
        let mut selection = Selection::default();
        selection.press_cell(2, 0, true);
        selection.release();
        selection.press_cell(0, 0, true);
        selection.release();
        assert_eq!(
            to_tsv(&rows(), &selection, &[0, 1, 2, 3]).as_deref(),
            Some("0\tname0\tx\n2\tname2\t"),
            "ascending by row, whatever order they were picked in"
        );
    }

    /// A searched table copies what is on screen: the rect holds display positions, and this
    /// maps them through the visible order rather than indexing the result directly.
    #[test]
    fn copying_follows_the_visible_order() {
        let mut selection = Selection::default();
        selection.press_cell(0, 0, false);
        selection.drag_to_cell(1, 0);
        // Only rows 3 and 1 are on screen, in that order.
        assert_eq!(
            to_tsv(&rows(), &selection, &[3, 1]).as_deref(),
            Some("3\n1")
        );
    }

    #[test]
    fn nothing_selected_copies_nothing() {
        assert_eq!(to_tsv(&rows(), &Selection::default(), &[0, 1, 2, 3]), None);
    }

    #[test]
    fn the_selection_describes_itself() {
        let mut selection = Selection::default();
        assert_eq!(describe(&selection), None);

        selection.press_cell(0, 0, false);
        assert_eq!(describe(&selection).as_deref(), Some("1 cell"));

        selection.drag_to_cell(1, 1);
        assert_eq!(describe(&selection).as_deref(), Some("4 cells"));

        selection.press_cell(0, 0, true);
        selection.release();
        assert_eq!(describe(&selection).as_deref(), Some("1 row"));
    }
}
