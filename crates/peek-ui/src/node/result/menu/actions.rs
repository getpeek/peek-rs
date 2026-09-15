//! What the result node's commands actually do, once a [`Scope`] has said what they act on.
//!
//! These are methods on the node rather than on `CanvasView` because they read the node's rows,
//! selection and query; the canvas handlers in `canvas/dispatch/result.rs` only find the table
//! and call in.

use gpui_kit::{App, ClipboardItem, Context, Window};
use peek_document::{ResultSet, VariableRow, VariableValue};

use super::super::ResultTable;
use super::rows;
use super::scope::Scope;

/// The three serialisations the menus offer, in the reference's own order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Json,
    Csv,
    Sql,
}

impl Format {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Csv => "CSV",
            Self::Sql => "SQL",
        }
    }

    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Sql => "sql",
        }
    }
}

impl ResultTable {
    /// What the live menu target and selection say a command should act on.
    pub(crate) fn scope(&self, cx: &App) -> Scope {
        let table = self.table.read(cx);
        super::scope::resolve(table.delegate().menu_target(), table.delegate().selection())
    }

    /// The scope's rows, cut out of the result.
    fn scoped_rows(&self, scope: &Scope, cx: &App) -> ResultSet {
        let table = self.table.read(cx);
        let delegate = table.delegate();
        rows::cut(delegate.result_rows(), scope, delegate.matches().visible())
    }

    /// One cell's value as plain text — `copyCellValue`, which copies the raw value and no
    /// formatting at all. A NULL copies as the empty string, so a spreadsheet cell arrives blank.
    pub(crate) fn copy_value(&self, cx: &mut Context<Self>) {
        let Scope::Cell(row, column) = self.scope(cx) else {
            return;
        };
        let table = self.table.read(cx);
        let Some(cell) = table.delegate().result_rows().cell(row, column) else {
            return;
        };
        let text = if cell.is_null() {
            String::new()
        } else {
            cell.to_display_string()
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// The scope, serialised. `None` when there is nothing to serialise, which is what keeps an
    /// empty result from putting a header-only file on the clipboard.
    pub(crate) fn serialize(&self, format: Format, cx: &App) -> Option<String> {
        let scope = self.scope(cx);
        let rows = self.scoped_rows(&scope, cx);
        if rows.is_empty() {
            return None;
        }
        Some(match format {
            Format::Json => peek_document::to_json(&rows),
            Format::Csv => peek_document::to_csv(&rows),
            Format::Sql => peek_db::mutation::build_insert(
                crate::database::Database::engine(cx),
                &self.export_table(),
                &rows,
            ),
        })
    }

    pub(crate) fn copy_as(&self, format: Format, cx: &mut Context<Self>) {
        if let Some(text) = self.serialize(format, cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// `(filename, contents)` for a save, or `None` when there is nothing to write.
    pub(crate) fn export_payload(&self, format: Format, cx: &App) -> Option<(String, String)> {
        let contents = self.serialize(format, cx)?;
        let scope = self.scope(cx);
        let table = self.table.read(cx);
        let base = peek_document::export_slug(&self.query);
        let name = rows::export_name(&base, &scope, table.delegate().result_rows().columns());
        Some((format!("{name}.{}", format.extension()), contents))
    }

    /// The table name SQL inserts are addressed to.
    ///
    /// `getExportTableName` is deliberately looser than the editable check: a join or an
    /// aggregate has no single writable table, but it still has a sensible name to export under.
    fn export_table(&self) -> String {
        self.tables.first().map_or_else(
            || peek_document::export_slug(&self.query),
            std::string::ToString::to_string,
        )
    }

    /// Spawns a variable node holding the scope's values, wired to this result.
    ///
    /// The two shapes are the reference's and the difference matters: a single cell keeps its
    /// **raw** value, while a column or a single-column rectangle is SQL-quoted, so the list
    /// drops straight into an `IN (…)` without the user quoting it by hand.
    pub(crate) fn use_as_variable(&mut self, cx: &mut Context<Self>) {
        let scope = self.scope(cx);
        let Some(column) = scope.variable_column() else {
            return;
        };
        let table = self.table.read(cx);
        let delegate = table.delegate();
        let Some(name) = delegate
            .result_rows()
            .columns()
            .get(column)
            .map(|column| column.name.clone())
        else {
            return;
        };

        let value = if let Scope::Cell(row, _) = scope {
            let cell = delegate.result_rows().cell(row, column);
            VariableValue::One(
                cell.map(peek_document::Cell::to_display_string)
                    .unwrap_or_default(),
            )
        } else {
            let engine = crate::database::Database::engine(cx);
            let cut = self.scoped_rows(&scope, cx);
            let sql_type = cut
                .columns()
                .first()
                .map_or_else(String::new, |column| column.sql_type.clone());
            VariableValue::Many(
                (0..cut.row_count())
                    .filter_map(|row| cut.cell(row, 0))
                    .map(|cell| peek_db::mutation::format_cell_literal(cell, &sql_type, engine))
                    .collect(),
            )
        };

        let node = self.node.clone();
        self.document.update(cx, |document, cx| {
            let placed = document.place_variable(&node, VariableRow { name, value });
            document.select_only([placed]);
            cx.notify();
        });
    }

    /// The menu's Delete, which is the toolbar's confirm dialog under another name.
    pub(crate) fn delete_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_delete(window, cx);
    }
}
