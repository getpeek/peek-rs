//! Option-drag to connect: the edge that follows the pointer out of its source node, and the
//! ring on the node it would land on.
//!
//! The reference drags out of small React Flow handles. Here the whole card is the source and
//! the whole target card is the drop zone, so the gesture needs no handle to aim for — which
//! makes the ring the only feedback telling a valid drop from a miss.

use gpui_kit::{App, Hsla};
use peek_canvas::edge::curve_between;
use peek_canvas::hit::connection_target;
use peek_canvas::{Rect, Size};
use peek_theme::{ActivePeekTheme, EdgeState};

use super::CanvasView;
use super::edges::{self, EdgeItem};

/// The pending edge and, when the pointer is over a node the drop would accept, that node's
/// world rect with the colour of the ring it is drawn with.
pub(super) struct Preview {
    pub edge: EdgeItem,
    pub target: Option<(Rect, Hsla)>,
}

impl CanvasView {
    pub(super) fn connection_preview(&self, cx: &App) -> Option<Preview> {
        let (source, screen) = self.interaction.connection_preview()?;
        let theme = cx.peek_theme();
        let page = self.document.read(cx).active_page();
        let source = page.node(source)?;
        let world = self.camera.screen_to_world(screen);
        let target = connection_target(page, &source.id, world).and_then(|id| page.node(&id));
        // Unsnapped, the curve ends at the pointer: a zero-size rect keeps its centre as the
        // intersection, so the line runs right up to the cursor.
        let end = target.map_or(
            Rect::new(world, Size::default()),
            peek_document::Node::bounds,
        );
        let kind = target.and_then(peek_document::Node::node_type);
        Some(Preview {
            edge: EdgeItem {
                curve: curve_between(source.bounds(), end),
                color: theme.edge(kind, EdgeState::ConnectionActive),
                width: edges::stroke_width(EdgeState::ConnectionActive),
            },
            target: target.map(|node| (node.bounds(), theme.selection_ring(kind))),
        })
    }
}
