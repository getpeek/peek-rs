//! Commands the canvas owns that no node does: the shell dialogs, the title-bar preference,
//! and the node clipboard behind cut, copy and paste.
//!
//! The palette dispatches through the canvas focus handle, so a handler anywhere below it is
//! never reached; opening a dialog or writing `settings.json` is a window-and-app call, which
//! the canvas can make as well as any view above it. That keeps `WorkspaceView` out of it. The
//! dialogs and the preference reach no view state and are registered as plain listeners; the
//! clipboard verbs need the document and the camera, so those go through `cx.listener`.

use gpui_kit::{App, Context, Global, InteractiveElement, Window};
use peek_canvas::Point;
use peek_config::Visibility;
use peek_document::geometry::Rect;
use peek_document::{Node, NodeId};

use super::CanvasView;
use crate::commands::actions;
use crate::settings::Settings;

/// The canvas clipboard: whole nodes, not text.
///
/// In-app, as the reference's `clipboardAtom` is, rather than the system pasteboard. A node is
/// a document object with no text form, and serialising one into the pasteboard would put JSON
/// in the way of every ordinary copy the user makes in another app. It also keeps the result
/// table's `cmd-c`, which *does* write TSV to the pasteboard, from fighting this one.
#[derive(Debug, Default)]
struct NodeClipboard(Vec<Node>);

impl Global for NodeClipboard {}

pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    element
        .on_action(|_: &actions::help::Keymap, window, cx| crate::keymap_help::open(window, cx))
        .on_action(|_: &actions::app::About, window, cx| crate::about::open(window, cx))
        .on_action(
            |_: &actions::settings::ToggleCommandPaletteButton, window, cx| {
                toggle_command_palette_button(window, cx);
            },
        )
        .on_action(
            |_: &actions::settings::ToggleAutomaticallyLabelQueries, _, cx| {
                toggle_automatic_query_labels(cx);
            },
        )
        .on_action(cx.listener(CanvasView::copy_selected_nodes))
        .on_action(cx.listener(CanvasView::cut_selected_nodes))
        .on_action(cx.listener(CanvasView::paste_nodes))
}

fn toggle_command_palette_button(window: &mut Window, cx: &mut App) {
    let next = if Settings::get(cx)
        .ui
        .titlebar
        .command_palette_button
        .is_shown()
    {
        Visibility::Hide
    } else {
        Visibility::Show
    };
    if let Err(error) = Settings::update(cx, |config| {
        config.ui.titlebar.command_palette_button = next;
    }) {
        // Read-only is the expected answer for most of this build's life, so this is not a
        // warning; the toggle still holds for the session either way.
        log::debug!("peek: title bar preference not saved: {error}");
    }

    // The title bar reads the global in `render` and belongs to `WorkspaceView`, which a canvas
    // listener has no handle on. Repainting the window is the whole fix. Temporary: an
    // `observe_global::<Settings>` on `WorkspaceView` is the narrower one, and `workspace.rs`
    // is not this file.
    window.refresh();
}

/// Turns AI query naming on or off. Nothing has to be repainted: the next run is what reads
/// the preference, and a name already written stays.
fn toggle_automatic_query_labels(cx: &mut App) {
    let next = !Settings::get(cx).ai.automatically_label_queries;
    if let Err(error) = Settings::update(cx, |config| {
        config.ai.automatically_label_queries = next;
    }) {
        log::debug!("peek: query label preference not saved: {error}");
    }
}

impl CanvasView {
    /// `cmd-c` on the canvas. The result table binds the same action on its own context and
    /// copies its cells as TSV instead; it sits deeper on the dispatch path and gpui stops
    /// propagation after the bubble phase by default, so a focused table wins and this never
    /// runs behind it.
    fn copy_selected_nodes(
        &mut self,
        _: &actions::edit::copy::Copy,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let copied = self.selected_nodes(cx);
        if copied.is_empty() {
            return;
        }
        cx.set_global(NodeClipboard(copied));
    }

    fn cut_selected_nodes(
        &mut self,
        _: &actions::edit::Cut,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cut = self.selected_nodes(cx);
        if cut.is_empty() {
            return;
        }
        let ids: Vec<NodeId> = cut.iter().map(|node| node.id.clone()).collect();
        cx.set_global(NodeClipboard(cut));
        self.document.update(cx, |document, cx| {
            if document.remove_nodes(&ids) > 0 {
                document.checkpoint();
                cx.notify();
            }
        });
    }

    fn paste_nodes(
        &mut self,
        _: &actions::edit::Paste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(clipboard) = cx.try_global::<NodeClipboard>() else {
            return;
        };
        let pasted = clipboard.0.clone();
        if pasted.is_empty() {
            return;
        }
        let delta = self.paste_translation(&pasted, window);
        self.document.update(cx, |document, cx| {
            document.paste_nodes(&pasted, delta);
            document.checkpoint();
            cx.notify();
        });
    }

    fn selected_nodes(&self, cx: &Context<Self>) -> Vec<Node> {
        let document = self.document.read(cx);
        document
            .selected()
            .iter()
            .filter_map(|id| document.node(id).cloned())
            .collect()
    }

    /// `pasteTranslation.ts`: the offset that lands the clipboard's bounding box in the middle
    /// of the visible pane, so a paste always arrives on screen rather than back at the
    /// coordinates it was copied from — which may be a page or a flight away.
    fn paste_translation(&self, nodes: &[Node], window: &Window) -> Point {
        let Some(bounds) = nodes.iter().map(Node::bounds).reduce(Rect::union) else {
            return Point::default();
        };
        let visible = self.camera.visible_world_rect(self.pane_size(window));
        let (from, to) = (bounds.center(), visible.center());
        Point::new(to.x - from.x, to.y - from.y)
    }
}
