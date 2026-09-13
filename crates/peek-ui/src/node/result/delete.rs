//! Deleting selected rows.
//!
//! Ported from `useCommitDelete.ts`. Like an edit, the statement comes from
//! `peek_db::mutation` and is pinned by primary key; unlike an edit, it can remove many rows at
//! once, so it asks first.
//!
//! **Not undoable, and the dialog says so.** The canvas history covers the document, not the
//! database — `cmd-z` after this brings nothing back.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, StyledExt, WindowExt};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, SharedString, Window, div, rems};
use peek_db::mutation::{self, NotEditable};

use super::ResultTable;
use super::editable;
use crate::database::Database;

impl ResultTable {
    /// How many rows the delete affordance offers to remove, and `None` when it should not be
    /// offered at all — nothing selected, no connection, or a result that cannot be written.
    pub(super) fn deletable_rows(&self, cx: &App) -> Option<usize> {
        if !Database::is_connected(cx) {
            return None;
        }
        self.editable_table(cx)?;
        let count = self
            .table
            .read(cx)
            .delegate()
            .selection()
            .selected_rows()
            .len();
        (count > 0).then_some(count)
    }

    /// Asks before deleting. The reference uses a modal for the same reason: this is the one
    /// action in the table that cannot be taken back.
    pub(super) fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(count) = self.deletable_rows(cx) else {
            return;
        };
        let table = self.editable_table(cx).unwrap_or_default();
        let this = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _, _| {
            let this = this.clone();
            let table = table.clone();
            dialog
                .title(SharedString::from(if count == 1 {
                    "Delete 1 row?".to_string()
                } else {
                    format!("Delete {count} rows?")
                }))
                .content(move |content, _, _| {
                    content.child(div().p(rems(0.5)).child(format!(
                        "This removes {} from \"{table}\" and cannot be undone.",
                        if count == 1 {
                            "1 row".to_string()
                        } else {
                            format!("{count} rows")
                        }
                    )))
                })
                .footer(
                    div()
                        .h_flex()
                        .gap(rems(0.375))
                        .child(
                            Button::new("result-delete-confirm")
                                .danger()
                                .small()
                                .label("Delete")
                                .on_click(move |_, window, cx| {
                                    if let Some(table) = this.upgrade() {
                                        table.update(cx, super::ResultTable::run_delete);
                                    }
                                    window.close_all_dialogs(cx);
                                }),
                        )
                        .child(
                            Button::new("result-delete-cancel")
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_all_dialogs(cx)),
                        ),
                )
        });
    }

    /// Builds and runs the `DELETE`, then re-runs the query behind the result.
    fn run_delete(&mut self, cx: &mut Context<Self>) {
        let statement = match self.build_delete(cx) {
            Ok(statement) => statement,
            Err(reason) => {
                log::error!("peek: cannot delete from {}: {reason}", self.node);
                return;
            }
        };
        let Some(session) = Database::session(cx) else {
            return;
        };
        let node = self.node.clone();
        let document = self.document.clone();

        cx.spawn(async move |this, cx| {
            let outcome = session.execute(statement).await;
            this.update(cx, |this, cx| {
                match outcome {
                    Ok(Ok(removed)) => {
                        log::info!("peek: deleted {removed} row(s) for {}", this.node);
                    }
                    Ok(Err(error)) => log::error!("peek: delete failed: {error}"),
                    Err(_) => log::error!("peek: the database runtime stopped"),
                }
                // Either way the rows on screen may no longer match the table, so refresh.
                crate::execution::rerun_source(&document, &node, cx);
            })
            .ok();
        })
        .detach();
    }

    fn build_delete(&self, cx: &App) -> Result<String, NotEditable> {
        let table_name = self
            .editable_table(cx)
            .ok_or(NotEditable::NotASingleTableSelect)?;
        let engine = Database::engine(cx);
        let state = self.table.read(cx);
        let delegate = state.delegate();
        let rows = delegate.result_rows();

        let keys = {
            let shared = crate::node::query::language::SqlLanguage::schema(cx);
            let schema = shared.read();
            schema
                .primary_keys
                .get(&table_name)
                .cloned()
                .unwrap_or_default()
        };

        // Selections hold display positions, so a searched table deletes the rows on screen.
        let visible = delegate.matches().visible();
        let bindings = delegate
            .selection()
            .selected_rows()
            .iter()
            .filter_map(|position| visible.get(*position).copied())
            .map(|row| editable::key_bindings(rows, row, (&table_name, &keys), engine))
            .collect::<Result<Vec<_>, _>>()?;

        mutation::build_delete(engine, &table_name, &keys, &bindings)
    }
}
