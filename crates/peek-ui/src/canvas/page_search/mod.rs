//! Page-wide node search: `~/labs/peek/src/page-search/PageSearch.tsx`.
//!
//! A panel over the canvas that fuzzy-finds any node on the active page by its contents. The
//! camera follows the highlighted row, so scanning the list doubles as wayfinding; confirming
//! selects the node and frames it.
//!
//! The list is gpui-component's `Command` with its own filtering switched off — the ranking,
//! the grouping by kind and the caps are [`corpus`]'s, which is the reference's `useNodeSearch`.

mod corpus;

use std::time::Duration;

use gpui_kit::TestSupportExt;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::{IndexPath, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement, SharedString, Window, div, rems,
};
use peek_canvas::Camera;
use peek_canvas::camera::FitOptions;
use peek_canvas::flight::durations;
use peek_document::NodeId;
use peek_theme::ActivePeekTheme;

use super::CanvasView;
use crate::commands::actions;
use crate::fuzzy::highlight;

use corpus::{Entry, Group};

/// The open overlay. Absent when page search is closed, which is the only state it has.
pub(super) struct PageSearch {
    state: Entity<CommandState>,
    /// Snapshotted when the panel opens: describing every node costs a walk over every result's
    /// first hundred rows, and a keystroke must not pay for it again.
    entries: Vec<Entry>,
    groups: Vec<Group>,
    /// The node the camera was last flown to, so a re-render that re-reports the same highlight
    /// does not restart the flight.
    flown_to: Option<NodeId>,
}

impl std::fmt::Debug for PageSearch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PageSearch")
            .field("groups", &self.groups.len())
            .finish_non_exhaustive()
    }
}

pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    element.on_action(cx.listener(CanvasView::open_page_search))
}

impl CanvasView {
    /// `Page::Search`. A result node holding focus handles this first and opens its own find bar
    /// instead; this is the fallback the palette and a canvas keypress reach.
    fn open_page_search(
        &mut self,
        _: &actions::page::Search,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.document.read(cx);
        let entries = corpus::entries(document.nodes(), |id| {
            document.result(id).map(std::convert::AsRef::as_ref)
        });
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |state, cx| state.focus(window, cx));
        self.page_search = Some(PageSearch {
            state,
            entries,
            groups: Vec::new(),
            flown_to: None,
        });
        cx.notify();
    }

    fn close_page_search(&mut self, cx: &mut Context<Self>) {
        if self.page_search.take().is_some() {
            cx.notify();
        }
    }

    /// The list follows the query, and so does the camera.
    ///
    /// `CommandState` reports a highlight change only when *it* moves the cursor, and a fresh
    /// query reinstalls the model rather than selecting through that path — so the first hit of
    /// a new query would never reach [`Self::track_page_search`]. Flying to it here is what makes
    /// typing, not only arrowing, move the camera.
    fn page_search_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.page_search.as_mut() else {
            return;
        };
        search.groups = corpus::search(&search.entries, query);
        search.flown_to = None;
        cx.notify();
        self.track_page_search(IndexPath::default(), window, cx);
    }

    fn page_search_hit(&self, path: IndexPath) -> Option<NodeId> {
        let search = self.page_search.as_ref()?;
        let group = search.groups.get(path.section)?;
        Some(group.hits.get(path.row)?.entry.id.clone())
    }

    /// The live camera: flying to whatever is highlighted is what makes scanning the list a way
    /// of finding things on the canvas.
    fn track_page_search(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.page_search_hit(path) else {
            return;
        };
        if self
            .page_search
            .as_ref()
            .and_then(|search| search.flown_to.as_ref())
            == Some(&id)
        {
            return;
        }
        if let Some(search) = self.page_search.as_mut() {
            search.flown_to = Some(id.clone());
        }
        self.frame_node(&id, durations::FIT_NODES, window, cx);
    }

    fn commit_page_search(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.page_search_hit(path) else {
            return;
        };
        self.close_page_search(cx);
        self.frame_node(&id, durations::FIT_NODES, window, cx);
    }

    /// Selects `id` alone and frames it, which is `selectOnly` + `fitNode` in the reference.
    /// Shared with the pivot handler, which frames the one node it reshaped.
    pub(super) fn frame_node(
        &mut self,
        id: &NodeId,
        duration: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self
            .document
            .read(cx)
            .node(id)
            .map(peek_document::Node::bounds)
        else {
            return;
        };
        let ids = [id.clone()];
        self.document.update(cx, |document, cx| {
            if document.select_only(ids) {
                cx.notify();
            }
        });
        let (pane, top) = self.framing_pane(window);
        let target = Self::below_chrome(
            Camera::fit_bounds(bounds, pane, FitOptions::padding(0.2)),
            top,
        );
        self.fly_to(target, duration, window, cx);
    }
}

/// The panel itself, rendered above the canvas and its chrome.
pub(super) fn render(view: &CanvasView, cx: &mut Context<CanvasView>) -> Option<AnyElement> {
    let search = view.page_search.as_ref()?;
    let state = search.state.clone();
    let groups: Vec<CommandGroup> = search
        .groups
        .iter()
        .map(|group| {
            CommandGroup::new()
                .label(group.node_type.label())
                .items(group.hits.iter().map(item))
        })
        .collect();
    let (radius, border, background) = {
        let theme = cx.peek_theme();
        (theme.radius_node, theme.node_border, theme.bg)
    };
    let canvas = cx.entity().downgrade();
    let (tracking, committing, cancelling) = (canvas.clone(), canvas.clone(), canvas);
    let querying = tracking.clone();

    let mut command = Command::new(&state)
        .bordered(false)
        .filterable(false)
        .placeholder("Search across every node…")
        .on_query(move |query, window, cx| {
            let query = query.to_string();
            querying
                .update(cx, |view, cx| view.page_search_query(&query, window, cx))
                .ok();
        })
        .on_select(move |path, window, cx| {
            tracking
                .update(cx, |view, cx| view.track_page_search(path, window, cx))
                .ok();
        })
        .on_confirm(move |path, window, cx| {
            committing
                .update(cx, |view, cx| view.commit_page_search(path, window, cx))
                .ok();
        })
        .on_cancel(move |_, cx| {
            cancelling.update(cx, CanvasView::close_page_search).ok();
        });
    for group in groups {
        command = command.group(group);
    }

    Some(
        div()
            .absolute()
            .top(rems(4.5))
            .left_0()
            .right_0()
            .h_flex()
            .justify_center()
            .child(
                div()
                    .id("page-search")
                    .test_support()
                    .occlude()
                    .w(rems(34.0))
                    .max_w_full()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .bg(background)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(command),
            )
            .into_any_element(),
    )
}

fn item(hit: &corpus::Hit) -> CommandItem {
    let label = SharedString::from(hit.entry.label.clone());
    let snippet = SharedString::from(hit.entry.snippet.clone());
    let indices = hit.label_match.clone();
    CommandItem::new()
        .label(label.clone())
        .child(move |_, cx| row(&label, &indices, &snippet, cx))
}

fn row(label: &SharedString, indices: &[usize], snippet: &SharedString, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    div()
        .v_flex()
        .min_w_0()
        .gap(rems(0.0625))
        .child(
            div()
                .h_flex()
                .min_w_0()
                .text_size(rems(0.8125))
                .text_color(theme.fg)
                .children(highlight(label, indices, cx)),
        )
        .child(
            div()
                .min_w_0()
                .text_ellipsis()
                .text_size(rems(0.6875))
                .text_color(theme.fg_subtle)
                .child(snippet.clone()),
        )
        .into_any_element()
}
