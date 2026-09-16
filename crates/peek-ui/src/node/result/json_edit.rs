//! Editing a JSON cell.
//!
//! The draft, its validity and the failure all live here, on the table that owns the commit
//! path; `canvas/json_editor.rs` only draws them. The split is what lets the panel be chrome —
//! pinned at rem 1 outside the camera's scope — without the canvas learning anything about
//! results, statements or primary keys.
//!
//! The commit itself is `edit.rs`'s, unchanged: the same `UPDATE … WHERE <pk> = …` and the same
//! re-run of the query behind the result. All this adds is a second place the draft can come
//! from, which is why [`super::delegate::Editing`] carries `popover`.

use gpui_kit::AppContext as _;
use gpui_kit::component::input::EditorState;
use gpui_kit::{App, Bounds, Context, Entity, Pixels, SharedString, Window};
use peek_document::Cell;

use super::ResultTable;
use crate::canvas::json_editor::JsonEditorState;

impl ResultTable {
    /// Opens a JSON cell: the popover when the result can be written to, the read-only value
    /// pane when it cannot.
    ///
    /// A cell you cannot save is a cell the editor would only mislead you about, and the pane
    /// reads better than a disabled field.
    pub(super) fn open_json_cell(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table().is_none() || !crate::database::Database::is_connected(cx) {
            self.table.update(cx, |table, cx| {
                table.delegate_mut().set_detail(Some((row, column)));
                cx.notify();
            });
            cx.notify();
            return;
        }
        self.begin_json_edit(row, column, window, cx);
    }

    /// Loads the cell into the editor and raises the panel.
    ///
    /// The draft opens pretty-printed, as `MonacoJsonCell` does: a jsonb value arrives from the
    /// driver on one line, and one line is not a document anyone can edit.
    fn begin_json_edit(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.table.read(cx);
        let delegate = state.delegate();
        let Some(value) = delegate.result_rows().cell(row, column) else {
            return;
        };
        let draft = match value {
            Cell::Json(json) => {
                serde_json::to_string_pretty(json).unwrap_or_else(|_| json.to_string())
            }
            other => other.to_display_string(),
        };
        let name = delegate
            .result_rows()
            .columns()
            .get(column)
            .map_or_else(SharedString::default, |column| {
                SharedString::from(column.name.clone())
            });

        self.json_editor.update(cx, |editor, cx| {
            editor.set_value(draft, window, cx);
            editor.focus(window, cx);
        });
        self.table.update(cx, |table, cx| {
            table.delegate_mut().begin_json_edit(row, column);
            cx.notify();
        });

        let Some(canvas) = self.canvas.upgrade() else {
            return;
        };
        let table = cx.entity().downgrade();
        canvas.update(cx, |canvas, cx| {
            canvas.open_json_editor(
                JsonEditorState {
                    table,
                    column: name,
                    // Zero until the anchor cell reports its own bounds on the next frame,
                    // which `report_json_anchor` does from the cell's prepaint.
                    anchor: Bounds::default(),
                },
                cx,
            );
        });
        cx.notify();
    }

    /// Tells the canvas where the anchor cell ended up.
    ///
    /// Called from the cell's own prepaint, every frame the popover is open, so the panel
    /// follows a pan, a zoom or a scroll of the rows. Only a change notifies: the cell reports
    /// on every frame and a repaint per report would never settle.
    pub(super) fn report_json_anchor(&self, bounds: Bounds<Pixels>, cx: &mut App) {
        let Some(canvas) = self.canvas.upgrade() else {
            return;
        };
        canvas.update(cx, |canvas, cx| canvas.move_json_editor(bounds, cx));
    }

    pub(crate) fn json_editor_state(&self) -> &Entity<EditorState> {
        &self.json_editor
    }

    /// Whether a statement is in flight, and why the last one failed.
    pub(crate) fn json_edit_status(&self, cx: &App) -> (bool, Option<SharedString>) {
        self.table
            .read(cx)
            .delegate()
            .editing()
            .map_or((false, None), |editing| {
                (
                    editing.saving,
                    editing.error.clone().map(SharedString::from),
                )
            })
    }

    /// Whether the draft would parse.
    ///
    /// An empty draft counts as valid: it commits to NULL, which is a legal value for a JSON
    /// column, and is how the NULL affordance works everywhere else in the table.
    pub(crate) fn json_draft_is_valid(&self, cx: &App) -> bool {
        let draft = self.json_editor.read(cx).value();
        let text = draft.trim();
        text.is_empty() || serde_json::from_str::<serde_json::Value>(text).is_ok()
    }

    /// Re-pretty-prints the draft, leaving anything that does not parse alone — the validity
    /// dot already says why nothing happened.
    pub(crate) fn format_json_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.json_editor.read(cx).value().to_string();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(draft.trim()) else {
            return;
        };
        let Ok(pretty) = serde_json::to_string_pretty(&value) else {
            return;
        };
        self.json_editor.update(cx, |editor, cx| {
            editor.set_value(pretty, window, cx);
            editor.focus(window, cx);
        });
        cx.notify();
    }

    /// The text the open editor holds, whichever of the two it is.
    pub(super) fn draft(&self, cx: &App) -> String {
        let popover = self
            .table
            .read(cx)
            .delegate()
            .editing()
            .is_some_and(|editing| editing.popover);
        if popover {
            return self.json_editor.read(cx).value().to_string();
        }
        self.table
            .read(cx)
            .delegate()
            .input()
            .read(cx)
            .value()
            .to_string()
    }
}

/// The popover's editor.
///
/// JSON highlighting costs nothing to switch on: gpui-component bundles `tree-sitter-json` with
/// its base `tree-sitter` feature and does not gate `LanguageName::Json` behind one of its own,
/// so unlike `sql` there is no grammar to register — only the `property` role the Peek themes
/// were missing for its object keys (`peek-theme/src/component_map.rs`).
pub(super) fn editor(window: &mut Window, cx: &mut App) -> Entity<EditorState> {
    cx.new(|cx| {
        EditorState::new(window, cx)
            .language("json")
            // The panel is 380 px of chrome beside a cell, not an IDE: line numbers and the
            // editor's own find bar would both cost more room than they are worth, and the
            // find bar would take cmd-f from the canvas as the query editor's would.
            .line_number(false)
            .searchable(false)
            .soft_wrap(true)
            .placeholder("null")
    })
}
