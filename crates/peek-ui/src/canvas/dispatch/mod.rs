//! Canvas-level handlers for commands that act on the *selection*.
//!
//! The command palette confirms through the canvas focus handle, and node elements are children
//! of the canvas, not ancestors of it — so an action handled only on a node view is listed by
//! the palette and then silently does nothing. Anything that can be invoked without the node
//! holding focus is therefore handled here and reads the selection instead, the way
//! `Agent::Fork` already did.
//!
//! A node keeps its own handler where the command must also fire *while* its editor has focus;
//! that handler is deeper on the dispatch path and stops propagation, so it wins and the
//! fallback below never runs.

mod execution;
mod layout;
mod navigation;
mod regions;
mod result;
mod shell;

use gpui_kit::{Context, InteractiveElement, Window};
use peek_document::{NodeId, NodeType};

use super::CanvasView;
use crate::commands::actions;
use crate::node::state::NodeState;

/// Generic over the element type: `.test_support()` wraps the canvas div in `Observed<_>`
/// under the test feature, so a concrete parameter would only compile in one configuration.
pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    let element = element
        .on_action(cx.listener(CanvasView::run_selected_queries))
        .on_action(cx.listener(CanvasView::format_selected_queries));
    let element = execution::register(element, cx);
    let element = navigation::register(element, cx);
    let element = result::register(element, cx);
    let element = layout::register(element, cx);
    let element = regions::register(element, cx);
    shell::register(element, cx)
}

impl CanvasView {
    /// The selected nodes of one kind, in document order.
    pub(super) fn selected_of_kind(&self, kind: NodeType, cx: &Context<Self>) -> Vec<NodeId> {
        let document = self.document.read(cx);
        document
            .selected()
            .iter()
            .filter(|id| {
                document
                    .node(id)
                    .and_then(peek_document::Node::node_type)
                    .is_some_and(|found| found == kind)
            })
            .cloned()
            .collect()
    }

    fn run_selected_queries(
        &mut self,
        _: &actions::query::Run,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.for_each_selected_query(window, cx, |state, _, cx| state.run(cx));
    }

    fn format_selected_queries(
        &mut self,
        _: &actions::query::Format,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.for_each_selected_query(window, cx, |state, window, cx| state.format(window, cx));
    }

    fn for_each_selected_query(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        work: impl Fn(&crate::node::query::QueryState, &mut Window, &mut Context<Self>),
    ) {
        for id in self.selected_of_kind(NodeType::Query, cx) {
            let Some(node) = self.document.read(cx).node(&id).cloned() else {
                continue;
            };
            if let Some(NodeState::Query(state)) = self.node_states.get(&node, window, cx) {
                work(state, window, cx);
            }
        }
    }
}
