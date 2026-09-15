//! What a context-menu command acts on.
//!
//! The reference splits this across the menu's own render (`CellContextMenu.tsx`, which decides
//! which items to show) and each handler (which decides what to read). Both are the same
//! question — *what is the target?* — so it is answered once here, and the menu and the commands
//! both read the answer.
//!
//! Pure index arithmetic, so every rule below is a plain unit test.

use super::super::selection::{CellRect, Selection};

/// Where the pointer was when the menu opened.
///
/// Carries **both** index spaces, as `CellMenuTarget` does: `row` indexes the result, and
/// `position` indexes the display. They are the same until a search re-sorts the rows, and the
/// two answer different questions — copying reads the data row, membership of the cell rectangle
/// is tested against the display position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuTarget {
    pub(crate) row: usize,
    pub(crate) position: usize,
    pub(crate) column: usize,
    /// A header press: the column is the target and there is no row.
    pub(crate) header: bool,
}

/// What the command should read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    /// A rectangle of cells.
    Cells(CellRect),
    /// Whole rows, by **data** index, ascending.
    Rows(Vec<usize>),
    /// Every row of one column.
    Column(usize),
    /// One cell, by data row.
    Cell(usize, usize),
    /// The result entire, which is what the palette means when nothing is picked out.
    Whole,
}

/// The target a menu opened on, or the live selection when there is none.
///
/// The precedence is `CellContextMenu.tsx`'s, including both of its quirks:
///
/// - a **1×1 rectangle degrades** to the single cell, so a stray click never puts the plural
///   "copy selection" wording in front of one value;
/// - the row branch needs the clicked row to actually be *in* the selection, so right-clicking
///   elsewhere acts on what was clicked and leaves the selection standing.
pub(crate) fn resolve(target: Option<MenuTarget>, selection: &Selection) -> Scope {
    let Some(target) = target else {
        return from_selection(selection);
    };
    if target.header {
        return Scope::Column(target.column);
    }
    if let Some(rect) = selection.rect()
        && rect.area() >= 2
        && rect.contains(target.position, target.column)
    {
        return Scope::Cells(rect);
    }
    let rows = selection.selected_rows();
    if rows.len() >= 2 && rows.contains(&target.row) {
        return Scope::Rows(rows.iter().copied().collect());
    }
    Scope::Cell(target.row, target.column)
}

/// What the palette means: there is no pointer, so the selection is the whole story.
fn from_selection(selection: &Selection) -> Scope {
    if let Some(rect) = selection.rect() {
        return Scope::Cells(rect);
    }
    let rows = selection.selected_rows();
    if rows.is_empty() {
        return Scope::Whole;
    }
    Scope::Rows(rows.iter().copied().collect())
}

impl Scope {
    /// The column a variable list would draw its values from, which needs there to be exactly one
    /// (`spawnVariableFromSelection` bails unless `cellRect.left === cellRect.right`).
    pub(crate) fn variable_column(&self) -> Option<usize> {
        match self {
            Self::Cells(rect) if rect.left == rect.right => Some(rect.left),
            Self::Column(column) | Self::Cell(_, column) => Some(*column),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MenuTarget, Scope, resolve};
    use crate::node::result::selection::Selection;

    fn cell(row: usize, column: usize) -> MenuTarget {
        MenuTarget {
            row,
            position: row,
            column,
            header: false,
        }
    }

    fn header(column: usize) -> MenuTarget {
        MenuTarget {
            row: 0,
            position: 0,
            column,
            header: true,
        }
    }

    #[test]
    fn with_nothing_selected_the_click_is_the_target() {
        let selection = Selection::default();
        assert_eq!(resolve(Some(cell(3, 1)), &selection), Scope::Cell(3, 1));
    }

    #[test]
    fn a_rectangle_the_click_is_inside_wins() {
        let mut selection = Selection::default();
        selection.press_cell(1, 0, false);
        selection.drag_to_cell(3, 1);

        let Scope::Cells(rect) = resolve(Some(cell(2, 1)), &selection) else {
            panic!("expected the rectangle");
        };
        assert_eq!((rect.top, rect.bottom, rect.left, rect.right), (1, 3, 0, 1));
    }

    /// Right-clicking away from the selection acts on what was clicked, and the selection is left
    /// standing — the behaviour three separate guards protect in the reference.
    #[test]
    fn a_click_outside_the_rectangle_targets_the_clicked_cell() {
        let mut selection = Selection::default();
        selection.press_cell(1, 0, false);
        selection.drag_to_cell(3, 1);

        assert_eq!(resolve(Some(cell(7, 0)), &selection), Scope::Cell(7, 0));
    }

    /// A single-cell rectangle is not a "selection" worth pluralising.
    #[test]
    fn a_one_by_one_rectangle_degrades_to_the_cell() {
        let mut selection = Selection::default();
        selection.press_cell(2, 1, false);

        assert_eq!(resolve(Some(cell(2, 1)), &selection), Scope::Cell(2, 1));
    }

    #[test]
    fn a_row_band_the_click_is_inside_wins() {
        let mut selection = Selection::default();
        selection.press_cell(1, 0, true);
        selection.drag_to_cell(3, 0);
        selection.release();

        assert_eq!(
            resolve(Some(cell(2, 1)), &selection),
            Scope::Rows(vec![1, 2, 3])
        );
    }

    /// One selected row is not a band: the plural copy and export items stay away, though the
    /// delete affordance still keys off the selection.
    #[test]
    fn a_single_selected_row_is_not_a_band() {
        let mut selection = Selection::default();
        selection.press_cell(2, 0, true);
        selection.release();

        assert_eq!(resolve(Some(cell(2, 1)), &selection), Scope::Cell(2, 1));
    }

    #[test]
    fn a_header_targets_its_whole_column() {
        let selection = Selection::default();
        assert_eq!(resolve(Some(header(2)), &selection), Scope::Column(2));
    }

    /// From the palette there is no pointer, so the selection answers — and with nothing selected
    /// the command still means something: the whole result.
    #[test]
    fn without_a_target_the_selection_answers() {
        let mut selection = Selection::default();
        assert_eq!(resolve(None, &selection), Scope::Whole);

        selection.press_cell(0, 0, true);
        selection.release();
        assert_eq!(resolve(None, &selection), Scope::Rows(vec![0]));
    }

    #[test]
    fn only_a_single_column_scope_can_become_a_variable() {
        let mut selection = Selection::default();
        selection.press_cell(0, 1, false);
        selection.drag_to_cell(4, 1);
        let narrow = resolve(Some(cell(2, 1)), &selection);
        assert_eq!(narrow.variable_column(), Some(1));

        selection.drag_to_cell(4, 2);
        let wide = resolve(Some(cell(2, 2)), &selection);
        assert_eq!(
            wide.variable_column(),
            None,
            "two columns have no single list of values to offer"
        );
    }
}
