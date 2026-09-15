//! Canvas layout: `View::Organize`, `View::Schema` and `Zoom::FitSelectionAndLock`.
//!
//! The maths is in [`peek_canvas::Layout`]; everything here is the animation loop around it and
//! the conversion from the live database schema into the nodes the schema page draws.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use gpui_kit::{Context, InteractiveElement, Window};
use peek_canvas::Layout;
use peek_document::TableDefinitionData;

use super::super::OrganizeRun;
use super::CanvasView;
use crate::commands::actions;
use crate::database::Database;

/// One simulation tick per 60 Hz frame, so a run takes the same wall time whatever rate the
/// window is repainting at — the rule camera flights already follow.
const TICK: Duration = Duration::from_millis(16);
/// Ceiling on catching up after a stall. Without it a window that was occluded for a second
/// would run the whole simulation in the frame it comes back and skip the animation entirely.
const MAX_TICKS_PER_FRAME: u32 = 4;

pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    element
        .on_action(cx.listener(CanvasView::organize))
        .on_action(cx.listener(CanvasView::view_schema))
        .on_action(cx.listener(CanvasView::fit_selection_and_lock))
}

impl CanvasView {
    /// `organizeCanvas.tsx`. Arranges the page by its edges, refusing on the schema page —
    /// that one is laid out by the command that builds it, and two simulations writing the same
    /// positions would fight over every tick.
    fn organize(
        &mut self,
        _: &actions::view::Organize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.document.read(cx).on_schema_page() {
            return;
        }
        self.start_layout(window, cx);
    }

    /// `viewSchema.tsx`: the `schema` page, rebuilt from the live schema and then laid out.
    ///
    /// The reference keeps a simulation running on that page forever so a dragged table
    /// reflows its neighbours. Here the layout runs once and stops, because a tick is a
    /// document mutation: a perpetual simulation would mean the page never settles, the
    /// autosave never quiesces, and every frame is an undo-coalescing window that never closes.
    fn view_schema(
        &mut self,
        _: &actions::view::Schema,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let schema = Database::schema(cx);
        let tables: Vec<TableDefinitionData> = schema
            .tables
            .iter()
            .map(|(table, columns)| TableDefinitionData {
                table: table.clone(),
                columns: columns.clone(),
            })
            .collect();
        let references = table_references(&schema);

        // `set_viewport` writes to whichever page is active, so the outgoing camera goes first.
        self.commit_viewport(cx);
        let created = self.document.update(cx, |document, cx| {
            document.open_schema_page();
            let created = document.rebuild_schema_tables(&tables, &references);
            cx.notify();
            created
        });
        self.adopt_active_page(cx);
        if created.is_empty() {
            return;
        }
        self.start_layout(window, cx);
    }

    /// `fitNodesToView.tsx`'s second command: tile the selection across the viewport, then
    /// hold the camera on it. The lock is set, never toggled, so a second press is a re-fit.
    fn fit_selection_and_lock(
        &mut self,
        _: &actions::zoom::FitSelectionAndLock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fit_selection(&actions::zoom::FitSelection, window, cx);
        if !self.camera_locked {
            self.toggle_camera_lock(&actions::view::ToggleCameraLock, window, cx);
        }
    }

    /// Starts a run over the active page, replacing whatever was running. Below two nodes there
    /// is nothing to arrange and the command is just a fit, as the reference has it.
    fn start_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.organize = None;
        let Some(layout) = Layout::of_active_page(self.document.read(cx)) else {
            self.fit_view(&actions::zoom::FitView, window, cx);
            return;
        };
        // Seals whatever edit was open, so the run is its own undo step rather than the tail of
        // the user's last drag.
        self.document
            .update(cx, |document, _| document.checkpoint());
        self.organize = Some(OrganizeRun {
            layout,
            last_tick: Instant::now(),
        });
        self.fit_view(&actions::zoom::FitView, window, cx);
        // The first tick comes from the next `render`, which is where animation frames may be
        // requested at all — `request_animation_frame` panics outside layout, prepaint or paint.
        cx.notify();
    }

    /// Advances the running layout, beside the camera flight and on the same wall clock.
    pub(crate) fn tick_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.organize.is_none() {
            return;
        }
        // Positions are written to whatever page is active, so a page switch mid-run has to
        // stop it rather than scatter the nodes of the page the user just moved to. The
        // reference needs no such guard: its `setNodes` resolves through `activePageAtom`, so a
        // tick landing after a page switch edits the node array of the page it *meant*, finds
        // none of its ids there and no-ops. `Document::set_position` has no such backstop.
        let active = self.document.read(cx).active_page_id().clone();
        let Some(run) = self.organize.as_mut() else {
            return;
        };
        if run.layout.page() != &active {
            self.organize = None;
            return;
        }

        let steps = due_steps(run.last_tick);
        if steps == 0 {
            window.request_animation_frame();
            return;
        }
        run.last_tick += TICK * steps;
        let mut refit = false;
        for _ in 0..steps {
            refit |= run.layout.step();
        }
        let finished = run.layout.is_finished();
        let layout = &run.layout;
        self.document.update(cx, |document, cx| {
            if layout.apply(document) {
                cx.notify();
            }
        });

        if finished {
            self.organize = None;
            self.document
                .update(cx, |document, _| document.checkpoint());
        }
        if refit || finished {
            self.fit_view(&actions::zoom::FitView, window, cx);
        }
        if !finished {
            window.request_animation_frame();
        }
    }
}

/// How many ticks the wall clock owes the simulation, capped.
fn due_steps(last_tick: Instant) -> u32 {
    let elapsed = last_tick.elapsed().as_millis() / TICK.as_millis();
    u32::try_from(elapsed)
        .unwrap_or(MAX_TICKS_PER_FRAME)
        .min(MAX_TICKS_PER_FRAME)
}

/// Table-to-table pairs from the schema's foreign keys, in `useSchemaForceLayout`'s direction:
/// from the table being referenced to the one referencing it. De-duplicated, because a
/// composite key produces one entry per column, and self-references are dropped.
fn table_references(schema: &peek_db::Schema) -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    let mut pairs = Vec::new();
    for (referenced, referencing) in &schema.references {
        let source = table_of(referenced);
        for qualified in referencing {
            let target = table_of(qualified);
            if source == target {
                continue;
            }
            let pair = (source.to_string(), target.to_string());
            if seen.insert(pair.clone()) {
                pairs.push(pair);
            }
        }
    }
    pairs
}

fn table_of(qualified: &str) -> &str {
    qualified.split('.').next().unwrap_or(qualified)
}

#[cfg(test)]
mod tests {
    use super::table_references;

    #[test]
    fn foreign_keys_become_one_pair_per_table_pair() {
        let mut schema = peek_db::Schema::default();
        schema.references.insert(
            "users.id".to_string(),
            vec!["orders.user_id".to_string(), "orders.buyer_id".to_string()],
        );
        schema
            .references
            .insert("nodes.id".to_string(), vec!["nodes.parent_id".to_string()]);

        assert_eq!(
            table_references(&schema),
            vec![("users".to_string(), "orders".to_string())],
            "a composite key is one edge, and a self-reference is none"
        );
    }
}
