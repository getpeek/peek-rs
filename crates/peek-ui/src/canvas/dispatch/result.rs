use gpui_kit::{Context, InteractiveElement, Window};
use peek_canvas::flight::durations;
use peek_document::NodeType;

use super::CanvasView;
use crate::commands::actions;

pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    element.on_action(cx.listener(CanvasView::pivot_results))
}

impl CanvasView {
    /// `Result::Pivot` flips every selected result between the table and the record view.
    fn pivot_results(
        &mut self,
        _: &actions::result::Pivot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.selected_of_kind(NodeType::Result, cx);
        let Some(only) = ids.first().filter(|_| ids.len() == 1).cloned() else {
            self.toggle_pivot(&ids, cx);
            return;
        };
        self.toggle_pivot(&ids, cx);
        // A pivoted node changes shape, so on its own it usually re-lays-out off screen. With
        // several selected the camera cannot follow them all, and the reference skips it too.
        self.frame_node(&only, durations::FIT_SELECTED, window, cx);
    }

    fn toggle_pivot(&mut self, ids: &[peek_document::NodeId], cx: &mut Context<Self>) {
        self.document.update(cx, |document, cx| {
            let changed = ids
                .iter()
                .filter(|id| crate::node::result::pivot::toggle(document, id))
                .count();
            if changed > 0 {
                cx.notify();
            }
        });
    }
}
