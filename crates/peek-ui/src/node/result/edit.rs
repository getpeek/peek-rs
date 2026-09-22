//! Committing an inline edit.
//!
//! Ported from `useCommitEdit.ts`. The statement is built by `peek_db::mutation`, which is
//! unit-tested on its own; this is the path that gathers what it needs and runs it.
//!
//! **These writes are not undoable.** They change the database, not the document, so the canvas
//! history never sees them — which is why every refusal is checked *before* anything is sent and
//! the statement is pinned to one row by its primary key.

use gpui_kit::{App, Context, Window};
use peek_db::mutation::{self, NotEditable};
use peek_document::Cell;

use super::ResultTable;
use super::editable;
use crate::database::Database;

impl ResultTable {
    /// Opens a cell for editing, unless the result is not editable — in which case the reason
    /// goes to the log rather than nowhere.
    pub(super) fn begin_edit(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Database::is_connected(cx) {
            return;
        }
        if self.editable_table().is_none() {
            log::info!(
                "peek: {} is not editable: {}",
                self.node,
                NotEditable::NotASingleTableSelect
            );
            return;
        }

        let draft = self
            .table
            .read(cx)
            .delegate()
            .result_rows()
            .cell(row, column)
            .map_or_else(String::new, Cell::to_display_string);
        self.table.update(cx, |table, cx| {
            table.delegate_mut().begin_edit(row, column);
            let input = table.delegate().input().clone();
            input.update(cx, |input, cx| {
                input.set_value(draft.clone(), window, cx);
                input.focus(window, cx);
            });
            cx.notify();
        });
        cx.notify();
    }

    /// Drops the open edit. Called by the canvas as it takes the JSON panel down, so it must
    /// not reach back for the panel itself — that would be a re-entrant update on the canvas.
    pub(crate) fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().end_edit();
            cx.notify();
        });
        cx.notify();
    }

    /// Drops the open edit *and* whatever surface was showing it.
    ///
    /// The node's own escape rule and every commit go through here; only the canvas, which has
    /// already taken the panel down, calls [`Self::cancel_edit`] directly.
    pub(super) fn dismiss_edit(&mut self, cx: &mut Context<Self>) {
        // This runs mid-update on the table, so the canvas drops the panel without reaching
        // back for it — `close_json_editor` would, and that double lease aborts the app on
        // every successful save. Cancelling the edit here is the half it leaves out.
        if let Some(canvas) = self.canvas.upgrade() {
            canvas.update(cx, crate::canvas::CanvasView::drop_json_editor);
        }
        self.cancel_edit(cx);
    }

    /// The table this result can be edited through.
    ///
    /// Read from the cache `reconcile` fills when the SQL changes, not parsed here: the toolbar
    /// asks on every frame to decide whether a delete button exists at all.
    pub(super) fn editable_table(&self) -> Option<&str> {
        self.editable.as_deref()
    }

    /// Builds the `UPDATE`, runs it, and re-runs the query behind the result.
    ///
    /// Every refusal happens before the database is touched.
    pub(crate) fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = self.table.read(cx).delegate().editing().cloned() else {
            return;
        };
        if editing.saving {
            return;
        }
        let statement = match self.build_update(&editing, cx) {
            Ok(statement) => statement,
            Err(reason) => {
                self.set_edit_error(Some(reason.to_string()), cx);
                return;
            }
        };
        let Some(session) = Database::session(cx) else {
            return;
        };

        self.set_edit_state(true, None, cx);
        let node = self.node.clone();
        let document = self.document.clone();

        cx.spawn(async move |this, cx| {
            let outcome = session.execute(statement).await;
            let failure = match outcome {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(error.to_string()),
                Err(_) => Some("the database runtime stopped".to_string()),
            };
            this.update(cx, |this, cx| {
                if let Some(message) = failure {
                    this.set_edit_state(false, Some(message), cx);
                    return;
                }
                this.dismiss_edit(cx);
                // Re-run the query behind the result rather than re-issuing the SQL by hand:
                // that re-resolves variables, re-places the rows and clears any error.
                crate::execution::rerun_source(&document, &node, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The statement for the open edit, or why there is not one.
    fn build_update(
        &self,
        editing: &super::delegate::Editing,
        cx: &App,
    ) -> Result<String, NotEditable> {
        let table_name = self
            .editable_table()
            .ok_or(NotEditable::NotASingleTableSelect)?;
        let engine = Database::engine(cx);
        // Whichever editor is open: the in-cell field, or the JSON popover.
        let draft = self.draft(cx);

        let state = self.table.read(cx);
        let delegate = state.delegate();
        let rows = delegate.result_rows();
        let column = rows
            .columns()
            .get(editing.column)
            .ok_or(NotEditable::NotASingleTableSelect)?;

        let keys = {
            let shared = crate::node::query::language::SqlLanguage::schema(cx);
            let schema = shared.read();
            schema
                .primary_keys
                .get(table_name)
                .cloned()
                .unwrap_or_default()
        };
        let bindings = editable::key_bindings(rows, editing.row, (table_name, &keys), engine)?;

        // An empty draft is NULL — how the NULL affordance works, without a separate flag.
        let literal = mutation::format_literal(&draft, &column.sql_type, engine);
        mutation::build_update(engine, (table_name, &column.name), &literal, &bindings)
    }

    fn set_edit_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.set_edit_state(false, error, cx);
    }

    fn set_edit_state(&mut self, saving: bool, error: Option<String>, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_edit_state(saving, error);
            cx.notify();
        });
        cx.notify();
    }

    /// A strip naming why the last commit failed, between the toolbar and the rows.
    ///
    /// The reference floats this under the cell it belongs to; inside a clipped, virtualised
    /// table there is nowhere to float it, so it goes where it is always visible instead.
    pub(super) fn edit_error(&self, cx: &App) -> Option<gpui_kit::AnyElement> {
        use gpui_kit::prelude::*;
        use peek_theme::ActivePeekTheme;

        let message = self
            .table
            .read(cx)
            .delegate()
            .editing()
            .and_then(|editing| editing.error.clone())?;
        let theme = cx.peek_theme();
        Some(
            gpui_kit::div()
                .flex_none()
                .w_full()
                .px(gpui_kit::rems(0.5))
                .py(gpui_kit::rems(0.25))
                .bg(theme.red_soft)
                .text_size(gpui_kit::rems(0.625))
                .text_color(theme.red)
                .child(message)
                .into_any_element(),
        )
    }
}
