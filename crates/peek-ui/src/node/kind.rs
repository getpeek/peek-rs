//! The one place that fans out to a per-kind module.
//!
//! Every kind owns a folder beside this file and appears here exactly once, so the kinds can
//! be built independently of each other: nothing but this dispatch and [`super::state`] is
//! shared between them.

use gpui_kit::{AnyElement, App, Entity, Window};
use peek_canvas::Document;
use peek_document::{Node, NodeKind};

use super::state::NodeState;
use super::{
    barchart, draw, placeholder, query, query_error, result, table_definition, text, variable,
};

/// What a body needs beyond its own data: somewhere to write back, the kind's retained state
/// when it has any, and whether the node is selected.
///
/// Bundled rather than passed as three more parameters so that the next thing a body needs —
/// region membership — does not churn every kind's signature again. `TableDefinition` renders
/// from its data alone and does not take it.
#[derive(Clone, Copy)]
pub(crate) struct NodeContext<'a> {
    pub(crate) document: &'a Entity<Document>,
    pub(crate) state: Option<&'a NodeState>,
    /// The shell draws its own selected border; a bare kind reads this to colour itself.
    pub(crate) selected: bool,
    /// The camera's zoom.
    ///
    /// Most kinds never need it: they are rem-based and the shell lays them out inside
    /// `with_rem_size(base * zoom)`, so they scale for free. A kind that has to size something
    /// in **pixels** does — and it cannot read the scaled rem size, because bodies are built in
    /// `CanvasView::render`, before the rem scope the element is later laid out in.
    pub(crate) zoom: f64,
}

impl std::fmt::Debug for NodeContext<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeContext")
            .field("has_state", &self.state.is_some())
            .field("selected", &self.selected)
            .field("zoom", &self.zoom)
            .finish_non_exhaustive()
    }
}

/// The node's header title.
pub(crate) fn title(node: &Node) -> String {
    match &node.kind {
        NodeKind::Query(data) => query::title(data),
        NodeKind::Text(data) => text::title(data),
        NodeKind::Variable(data) => variable::title(data),
        NodeKind::Barchart(data) => barchart::title(data),
        NodeKind::TableDefinition(data) => table_definition::title(data),
        NodeKind::QueryError(data) => query_error::title(data),
        NodeKind::Draw(data) => draw::title(data),
        NodeKind::Result(data) => result::title(data),
        _ => placeholder::title(node),
    }
}

/// The node's body, filling the shell below the header.
pub(crate) fn body(
    node: &Node,
    context: NodeContext<'_>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match &node.kind {
        NodeKind::Query(data) => query::body(&node.id, data, context, window, cx),
        NodeKind::Text(data) => text::body(&node.id, data, context, window, cx),
        NodeKind::Variable(data) => variable::body(&node.id, data, context, window, cx),
        NodeKind::Barchart(data) => barchart::body(&node.id, data, context, window, cx),
        NodeKind::QueryError(data) => query_error::body(&node.id, data, context, window, cx),
        NodeKind::Result(data) => result::body(&node.id, data, context, window, cx),
        NodeKind::Draw(data) => draw::body(node, data, context, cx),
        // Renders from its data alone, so it needs no context at all.
        NodeKind::TableDefinition(data) => table_definition::body(data, cx),
        _ => placeholder::body(node, cx),
    }
}

/// Controls a kind adds to the right of its header, if any.
pub(crate) fn header_extras(
    node: &Node,
    context: NodeContext<'_>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    match &node.kind {
        NodeKind::Query(data) => Some(query::header_extras(&node.id, data, context, window, cx)),
        NodeKind::Barchart(data) => barchart::header_extras(&node.id, data, context, window, cx),
        NodeKind::Variable(data) => variable::header_extras(&node.id, data, context, window, cx),
        _ => None,
    }
}

/// Whether a kind draws its own card instead of the shared shell. `TextNode.tsx` and
/// `DrawNode.tsx` have no header and no indicator.
pub(crate) fn is_bare(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Text(_) | NodeKind::Draw(_))
}
