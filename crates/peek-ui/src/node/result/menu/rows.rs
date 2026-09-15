//! Cutting a [`Scope`] out of a result, and naming the file it would be written to.
//!
//! The menus all serialise the same way and differ only in *what* they serialise, so the cut
//! happens once here and the three formats take the piece they are given.

use peek_document::{Column, ResultSet};

use super::scope::Scope;

/// The rows and columns a scope covers, as a result set in its own right.
///
/// `visible` maps display positions onto rows of the result, so a rectangle drawn over a searched
/// table cuts what is on screen rather than whatever sits at those indices — the rule `to_tsv`
/// already follows for `cmd-c`.
pub(crate) fn cut(rows: &ResultSet, scope: &Scope, visible: &[usize]) -> ResultSet {
    match scope {
        Scope::Whole => rows.clone(),
        Scope::Cells(rect) => {
            let columns = slice_columns(rows, rect.left, rect.right);
            let data = rect
                .rows()
                .filter_map(|position| visible.get(position).copied())
                .map(|row| slice_row(rows, row, rect.left, rect.right))
                .collect();
            ResultSet::new(columns, data)
        }
        Scope::Rows(indices) => {
            let last = rows.column_count().saturating_sub(1);
            let data = indices
                .iter()
                .map(|row| slice_row(rows, *row, 0, last))
                .collect();
            ResultSet::new(rows.columns().to_vec(), data)
        }
        Scope::Column(column) => {
            let columns = slice_columns(rows, *column, *column);
            let data = (0..rows.row_count())
                .map(|row| slice_row(rows, row, *column, *column))
                .collect();
            ResultSet::new(columns, data)
        }
        // A clicked cell means the whole **row** here: `Copy row` and `Export row` are what the
        // menu offers in that mode, and `Copy "value"` reads the one cell for itself.
        Scope::Cell(row, _) => {
            let last = rows.column_count().saturating_sub(1);
            ResultSet::new(
                rows.columns().to_vec(),
                vec![slice_row(rows, *row, 0, last)],
            )
        }
    }
}

fn slice_columns(rows: &ResultSet, left: usize, right: usize) -> Vec<Column> {
    rows.columns()
        .iter()
        .skip(left)
        .take(right.saturating_sub(left) + 1)
        .cloned()
        .collect()
}

fn slice_row(rows: &ResultSet, row: usize, left: usize, right: usize) -> Vec<peek_document::Cell> {
    (left..=right)
        .map(|column| {
            rows.cell(row, column)
                .cloned()
                .unwrap_or(peek_document::Cell::Null)
        })
        .collect()
}

/// What the saved file is called, before its extension.
///
/// `useRowActions.ts`' suffixes, so an export says what it was cut from: a row is 1-based
/// because that is how the user counted it, and a rectangle carries its shape.
pub(crate) fn export_name(base: &str, scope: &Scope, columns: &[Column]) -> String {
    match scope {
        Scope::Whole => base.to_string(),
        Scope::Cells(rect) => format!(
            "{base}-selection-{}x{}",
            rect.bottom - rect.top + 1,
            rect.right - rect.left + 1
        ),
        Scope::Rows(indices) => format!("{base}-{}-rows", indices.len()),
        Scope::Cell(row, _) => format!("{base}-row-{}", row + 1),
        Scope::Column(column) => columns
            .get(*column)
            .map_or_else(|| base.to_string(), |column| column.name.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::{cut, export_name};
    use crate::node::result::menu::scope::Scope;
    use crate::node::result::selection::CellRect;
    use peek_document::{Cell, Column, ResultSet};

    fn rows() -> ResultSet {
        ResultSet::new(
            vec![
                Column::new("id", "INT4"),
                Column::new("name", "TEXT"),
                Column::new("total", "INT8"),
            ],
            (0..4)
                .map(|index| {
                    vec![
                        Cell::Int(index),
                        Cell::Text(format!("row {index}")),
                        Cell::Int(index * 10),
                    ]
                })
                .collect(),
        )
    }

    fn visible() -> Vec<usize> {
        (0..4).collect()
    }

    #[test]
    fn a_rectangle_cuts_its_own_columns_and_rows() {
        let cut = cut(
            &rows(),
            &Scope::Cells(CellRect {
                top: 1,
                bottom: 2,
                left: 1,
                right: 2,
            }),
            &visible(),
        );
        assert_eq!(cut.column_count(), 2);
        assert_eq!(cut.row_count(), 2);
        assert_eq!(cut.columns()[0].name, "name");
        assert_eq!(cut.cell(0, 0), Some(&Cell::Text("row 1".to_string())));
    }

    /// A rectangle drawn over a searched table must cut what is on screen, not the rows that
    /// happen to sit at those indices in the result.
    #[test]
    fn a_rectangle_reads_through_the_visible_order() {
        let cut = cut(
            &rows(),
            &Scope::Cells(CellRect {
                top: 0,
                bottom: 1,
                left: 0,
                right: 0,
            }),
            &[3, 1],
        );
        assert_eq!(cut.cell(0, 0), Some(&Cell::Int(3)));
        assert_eq!(cut.cell(1, 0), Some(&Cell::Int(1)));
    }

    #[test]
    fn a_column_cut_keeps_every_row() {
        let cut = cut(&rows(), &Scope::Column(2), &visible());
        assert_eq!(cut.column_count(), 1);
        assert_eq!(cut.row_count(), 4);
        assert_eq!(cut.columns()[0].name, "total");
    }

    /// A clicked cell is the whole row: the menu's copy and export items say "row" there, and
    /// only `Copy "value"` is about the one cell.
    #[test]
    fn a_clicked_cell_cuts_its_whole_row() {
        let cut = cut(&rows(), &Scope::Cell(2, 1), &visible());
        assert_eq!((cut.row_count(), cut.column_count()), (1, 3));
        assert_eq!(cut.cell(0, 1), Some(&Cell::Text("row 2".to_string())));
    }

    #[test]
    fn rows_keep_every_column() {
        let cut = cut(&rows(), &Scope::Rows(vec![0, 3]), &visible());
        assert_eq!((cut.row_count(), cut.column_count()), (2, 3));
        assert_eq!(cut.cell(1, 0), Some(&Cell::Int(3)));
    }

    #[test]
    fn export_names_say_what_was_cut() {
        let columns = rows().columns().to_vec();
        assert_eq!(
            export_name("orders", &Scope::Cell(4, 0), &columns),
            "orders-row-5",
            "1-based, because that is how the row was counted on screen"
        );
        assert_eq!(
            export_name("orders", &Scope::Rows(vec![1, 2, 3]), &columns),
            "orders-3-rows"
        );
        assert_eq!(
            export_name(
                "orders",
                &Scope::Cells(CellRect {
                    top: 0,
                    bottom: 2,
                    left: 0,
                    right: 1
                }),
                &columns
            ),
            "orders-selection-3x2"
        );
        assert_eq!(
            export_name("orders", &Scope::Column(1), &columns),
            "name",
            "a column exports under its own name"
        );
    }
}
