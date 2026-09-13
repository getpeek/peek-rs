//! Going somewhere: to another page, to the next query on this one, or to the pages picker.
//!
//! They live on the canvas rather than on the title bar because the palette dispatches through
//! the canvas focus handle — see this module's parent. `ConnectionPicker::Open` is the one that
//! does not: switching a connection rebuilds the document and everything hanging off it, which
//! only `WorkspaceView` can reach, so it is handled there.

use gpui_kit::{App, Context, InteractiveElement, Window};
use peek_config::PageDisplay;
use peek_document::NodeType;

use super::CanvasView;
use crate::commands::actions;
use crate::settings::Settings;
use crate::title_bar::pages::PagesMenu;

pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    element
        .on_action(cx.listener(CanvasView::go_to_page))
        .on_action(cx.listener(CanvasView::select_previous_query))
        .on_action(cx.listener(CanvasView::select_next_query))
        // Closures rather than methods: neither of these reads the canvas, only the settings
        // global, and an `&mut self` they never touch is a lint and a lie about what they do.
        .on_action(cx.listener(|_, _: &actions::page::OpenPicker, _, cx| open_pages_picker(cx)))
        .on_action(
            cx.listener(|_, _: &actions::settings::TogglePageDisplay, _, cx| {
                toggle_page_display(cx);
            }),
        )
}

/// The reference's `o` only opens the picker in `list` display mode — in `tabs` mode every page
/// is already on screen and the popover would be a second way to see the same row.
fn open_pages_picker(cx: &mut App) {
    if Settings::get(cx).ui.pages.show_as == PageDisplay::List {
        PagesMenu::toggle(cx);
    }
}

fn toggle_page_display(cx: &mut App) {
    let next = match Settings::get(cx).ui.pages.show_as {
        PageDisplay::Tabs => PageDisplay::List,
        PageDisplay::List => PageDisplay::Tabs,
    };
    if let Err(error) = Settings::update(cx, |config| config.ui.pages.show_as = next) {
        // Read-only is the expected answer for most of this build's life, so this is not a
        // warning; the change still holds for the session either way.
        log::debug!("peek: page display preference not saved: {error}");
    }
    // Leaving list mode with the picker up would strand it: nothing renders its trigger.
    PagesMenu::close(cx);
}

impl CanvasView {
    fn go_to_page(&mut self, action: &actions::page::GoTo, _: &mut Window, cx: &mut Context<Self>) {
        self.switch_to_page(&action.page, cx);
    }

    fn select_previous_query(
        &mut self,
        _: &actions::page::SelectPreviousQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_query_selection(-1, window, cx);
    }

    fn select_next_query(
        &mut self,
        _: &actions::page::SelectNextQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_query_selection(1, window, cx);
    }

    /// Cycles the selection through the page's query nodes left to right, wrapping at both ends.
    ///
    /// Nothing selected — or something selected that is not a query — anchors before the first,
    /// so the next key lands on the leftmost query and the previous key on the rightmost. That
    /// is the reference's `idx = -1`, and it is why the arithmetic is signed.
    fn step_query_selection(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.document.read(cx);
        let mut queries: Vec<&peek_document::Node> = document
            .nodes()
            .iter()
            .filter(|node| node.node_type() == Some(NodeType::Query))
            .collect();
        if queries.is_empty() {
            return;
        }
        queries.sort_by(|left, right| left.position.x.total_cmp(&right.position.x));

        let current = queries
            .iter()
            .position(|node| document.is_selected(&node.id))
            .map_or(-1, |index| isize::try_from(index).unwrap_or(-1));
        let count = isize::try_from(queries.len()).unwrap_or(1);
        let target = (current + direction).rem_euclid(count);
        let Some(id) = queries
            .get(usize::try_from(target).unwrap_or(0))
            .map(|node| node.id.clone())
        else {
            return;
        };
        // The reference lands at zoom 1 rather than keeping the current zoom, unlike the arrow
        // keys: cycling queries is for reading one, and a query too small to read is no answer.
        self.go_to(&id, 1.0, window, cx);
    }
}
