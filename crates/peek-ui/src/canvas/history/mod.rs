//! Version history on the canvas: the timeline panel, the preview of a past version, and the
//! restore — `src/canvas/history/` in the reference.
//!
//! A preview never touches the live document. The canvas renders a detached copy of the page
//! as it was, with a node-state map of its own so the copy's editors are not the live ones;
//! the live map waits aside and comes back untouched when the preview ends. While the panel is
//! open the canvas publishes [`crate::commands::CANVAS_HISTORY`] instead of `Canvas`, so no
//! canvas binding fires, and a press on the board only pans.

mod panel;
mod store;

use std::collections::HashSet;

use gpui_kit::TestSupportExt;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{App, BoxShadow, Context, Entity, SharedString, Window, div, point, px};
use peek_canvas::camera::FitOptions;
use peek_canvas::flight::durations;
use peek_canvas::{Camera, Document, Size};
use peek_document::history::{HistoryFile, format};
use peek_document::{CanvasDocument, CheckpointId, DocVersion, Page, ResultSidecar};
use peek_theme::ActivePeekTheme;

use super::CanvasView;
use crate::commands::actions;
use crate::node::state::NodeStates;

pub(crate) use panel::{HistoryPanel, PanelEvent};
pub(crate) use store::VersionHistory;

/// A past version on screen in place of the live page.
pub(super) struct Preview {
    seq: u32,
    taken_at: i64,
    document: Entity<Document>,
    /// The live canvas's retained node state, set aside while the preview's own map is in use.
    live_states: NodeStates,
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Preview")
            .field("seq", &self.seq)
            .finish_non_exhaustive()
    }
}

impl CanvasView {
    /// The document the canvas is drawing: the preview's while one is up, else the live one.
    pub(crate) fn shown(&self) -> &Entity<Document> {
        self.preview
            .as_ref()
            .map_or(&self.document, |preview| &preview.document)
    }

    pub(crate) fn history_open(&self, cx: &App) -> bool {
        self.history_panel.read(cx).is_open()
    }

    /// Backs this canvas's history with a log on disk. A canvas starts with one in memory,
    /// which is all a test's workspace has.
    pub(crate) fn use_history_file(&mut self, file: HistoryFile, cx: &mut Context<Self>) {
        let document = self.document.clone();
        self.history = cx.new(|cx| VersionHistory::new(document, Some(file), cx));
    }

    /// Captures whatever the debounce is still holding, before the canvas goes away.
    pub(crate) fn flush_history(&mut self, cx: &mut Context<Self>) {
        self.history.update(cx, VersionHistory::flush);
    }

    fn toggle_history(
        &mut self,
        _: &actions::history::Toggle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history_open(cx) {
            self.close_history(window, cx);
            return;
        }
        self.jump = None;
        self.context_menu = None;
        self.regions
            .update(cx, |regions, cx| regions.close(window, cx));
        let history = self.history.clone();
        let page = self.document.read(cx).active_page_id().clone();
        self.history_panel
            .update(cx, |panel, cx| panel.open(history, page, window, cx));
        cx.notify();
    }

    /// Closes the timeline, for the surfaces that take over from it — the palette, whose
    /// commands must never run against a preview.
    pub(crate) fn close_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history_panel
            .update(cx, |panel, cx| panel.close(window, cx));
        self.end_preview(cx);
    }

    pub(super) fn on_history_event(
        &mut self,
        _: &Entity<HistoryPanel>,
        event: &PanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PanelEvent::Preview(entry) => {
                self.preview_version(entry.as_ref(), cx);
                self.fit_shown(window, cx);
            }
            PanelEvent::Restore(entry) => self.restore_version(entry, window, cx),
            PanelEvent::Closed => self.end_preview(cx),
        }
    }

    /// Follows a page switch under an open panel: the old page's preview is meaningless there.
    pub(super) fn history_follow_page(&mut self, cx: &mut Context<Self>) {
        if !self.history_open(cx) {
            return;
        }
        let page = self.document.read(cx).active_page_id().clone();
        if self.history_panel.read(cx).page() == Some(&page) {
            return;
        }
        self.end_preview(cx);
        self.history_panel
            .update(cx, |panel, cx| panel.follow_page(page, cx));
    }

    fn preview_version(&mut self, entry: Option<&CheckpointId>, cx: &mut Context<Self>) {
        let Some(entry) = entry else {
            self.end_preview(cx);
            return;
        };
        let Some((copy, rows, (seq, taken_at))) = self.version_copy(entry, cx) else {
            return;
        };
        // Result rows are session state the snapshot never held; the live ones are what the
        // reference shows under a preview too.
        let document = cx.new(|_| {
            let mut document = Document::detached(copy);
            document.adopt_results(rows);
            document
        });

        let states = NodeStates::new(document.clone(), cx.entity().downgrade());
        let previous = std::mem::replace(&mut self.node_states, states);
        let live_states = match self.preview.take() {
            // Scrubbing from one version to the next: the live map is still set aside, and
            // the map being replaced is the last preview's.
            Some(earlier) => {
                release(previous, cx);
                earlier.live_states
            }
            None => previous,
        };
        self.document.update(cx, |document, cx| {
            if document.deselect_all() {
                cx.notify();
            }
        });
        self.interaction = peek_canvas::gesture::Interaction::Idle;
        self.preview = Some(Preview {
            seq,
            taken_at,
            document,
            live_states,
        });
        cx.notify();
    }

    /// The live page with its content put back to `entry`, as a one-page document, plus the
    /// live rows and the entry's number and time.
    fn version_copy(
        &self,
        entry: &CheckpointId,
        cx: &App,
    ) -> Option<(CanvasDocument, ResultSidecar, (u32, i64))> {
        let live = self.document.read(cx);
        let page_id = live.active_page_id().clone();
        let history = self.history.read(cx);
        let seen = history
            .entries(&page_id)
            .iter()
            .find(|seen| &seen.id == entry)?;
        let snapshot = history.reconstruct(&page_id, entry)?;
        let current = live.active_page();
        let page = Page {
            id: page_id.clone(),
            name: current.name.clone(),
            viewport: current.viewport,
            nodes: snapshot.nodes,
            edges: snapshot.edges,
            regions: snapshot.regions,
        };
        let copy = CanvasDocument {
            version: DocVersion,
            active_page_id: page_id.clone(),
            page_order: vec![page_id.clone()],
            pages: [(page_id, page)].into(),
        };
        Some((copy, live.results().clone(), (seen.seq, seen.taken_at)))
    }

    fn end_preview(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.preview.take() else {
            return;
        };
        let states = std::mem::replace(&mut self.node_states, preview.live_states);
        release(states, cx);
        cx.notify();
    }

    /// Restores on top of history, never over it: pending edits become their own checkpoint
    /// first, the restore is recorded as a labelled one after, and it is a single undo step
    /// like any other edit.
    fn restore_version(
        &mut self,
        entry: &CheckpointId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = self.document.read(cx).active_page_id().clone();
        let history = self.history.read(cx);
        let Some(seq) = history
            .entries(&page)
            .iter()
            .find(|seen| &seen.id == entry)
            .map(|seen| seen.seq)
        else {
            return;
        };
        let Some(snapshot) = history.reconstruct(&page, entry) else {
            return;
        };
        self.history
            .update(cx, |history, cx| history.capture(&page, None, cx));
        self.end_preview(cx);
        self.document.update(cx, |document, cx| {
            if document.restore_page(snapshot) {
                cx.notify();
            }
        });
        let label = format!("Restored Version {seq}");
        let restored = self
            .history
            .update(cx, |history, cx| history.capture(&page, Some(label), cx));
        self.history_panel
            .update(cx, |panel, cx| panel.restored(restored, seq, cx));
        self.fit_shown(window, cx);
    }

    /// Frames whatever is on screen, above the panel, as `fitView` does after each selection.
    fn fit_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.shown().read(cx).content_bounds() else {
            return;
        };
        let (pane, top) = self.framing_pane(window);
        let covered = f64::from(panel::HEIGHT + panel::INSET * 2.0);
        let pane = Size::new(pane.width, (pane.height - covered).max(1.0));
        let target =
            Self::below_chrome(Camera::fit_bounds(bounds, pane, FitOptions::default()), top);
        self.fly_to(target, durations::FIT_VIEW, window, cx);
    }
}

/// Lets a node-state map give back what it holds outside itself before it is dropped.
fn release(mut states: NodeStates, cx: &mut App) {
    states.retain_live(&HashSet::new(), u64::MAX, cx);
}

/// `History::Toggle`, the preview's ring over the board, and the timeline over that.
pub(super) fn layers<E: ParentElement + InteractiveElement>(
    view: &CanvasView,
    element: E,
    cx: &mut Context<CanvasView>,
) -> E {
    element
        .on_action(cx.listener(CanvasView::toggle_history))
        .children(preview_ring(view, cx))
        .child(view.history_panel.clone())
}

/// `PreviewChrome`: an inset accent ring and a chip saying which version is on screen, so a
/// past version is never mistaken for the present. Pointer-transparent.
fn preview_ring(view: &CanvasView, cx: &App) -> Option<impl IntoElement> {
    let preview = view.preview.as_ref()?;
    let theme = cx.peek_theme();
    let label = SharedString::from(format!(
        "Previewing · Version {} · {}",
        preview.seq,
        format::stamp(preview.taken_at)
    ));
    Some(
        div()
            .id("history-preview")
            .test_support()
            .absolute()
            .inset_0()
            .border_2()
            .border_color(theme.accent_line)
            .shadow(vec![BoxShadow {
                color: theme.accent_bg,
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(70.0),
                spread_radius: px(0.0),
                inset: true,
            }])
            .child(
                div()
                    .absolute()
                    .top(panel::CHIP_TOP)
                    .left_0()
                    .right_0()
                    .h_flex()
                    .justify_center()
                    .child(
                        div()
                            .id("history-preview-chip")
                            .h_flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(10.0))
                            .py(px(4.0))
                            .rounded(theme.radius_pill)
                            .bg(theme.accent)
                            .text_color(theme.bg)
                            .text_size(px(11.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(
                                gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Clock)
                                    .size(px(11.0)),
                            )
                            .child(label),
                    ),
            ),
    )
}
