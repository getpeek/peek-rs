//! The pages picker: the panel under the title bar's pages pill, in `list` display mode.
//!
//! Ported from `~/labs/peek/src/components/titlebar/PageSelector/PagesMenu.tsx`, with the
//! reference's arrow/Enter cursor and one thing it does not have: a search box. The reference
//! can afford to leave it out because its list is a plain popover over a handful of pages; a
//! document that has grown past a screenful needs the same fuzzy find every other surface in
//! this app offers, and `Page::OpenPicker` is the only way to reach a page by name.
//!
//! It is a sibling of [`super::PageTabs`] rather than a popover inside it, for two reasons the
//! connection picker already ran into:
//!
//! - **A search box must own focus.** gpui-component's `Popover` binds `space` and `enter` to
//!   `Confirm` on the context it wraps its content in, so a field inside one never sees a space
//!   — the popover closes instead.
//! - **A scrim needs the whole window.** The panel dismisses on a press anywhere outside it,
//!   and that press must not also reach the canvas and start a marquee. Only a full-window
//!   sibling can swallow it, which is why `WorkspaceView` renders this layer rather than the
//!   title bar.

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, BoxShadow, Context, Entity, FocusHandle, KeyDownEvent, MouseButton,
    MouseDownEvent, Pixels, ScrollHandle, SharedString, Subscription, Window, div, point, px,
    transparent_black,
};
use peek_document::PageId;
use peek_theme::ActivePeekTheme;

use super::search::{self, PageRow};
use super::{PagesMenu, add_button, close_button};
use crate::canvas::CanvasView;

/// Narrower than the connection picker's 460: a page has one short name and no second line.
const WIDTH: Pixels = px(260.0);
/// The gap under the pill, the reference's `my={8}`.
const GAP_UNDER_PILL: Pixels = px(8.0);
/// Enough for a dozen rows before the list scrolls; shorter than the connection picker's 70vh,
/// which is sized for a tree rather than a flat list.
const MAX_HEIGHT: Pixels = px(360.0);
const DOT: Pixels = px(7.0);

pub(crate) struct PagesPanel {
    canvas: Entity<CanvasView>,
    /// Chrome dispatches through the canvas, so buttons take the same path the keyboard does.
    canvas_focus: FocusHandle,
    query: Entity<InputState>,
    /// An index into the *filtered* rows, so it means the same thing the list shows.
    cursor: usize,
    /// So a cursor arrowed past the bottom brings the list with it — the reference's
    /// `scrollIntoView({ block: "nearest" })`.
    rows_scroll: ScrollHandle,
    /// Whatever held focus when the panel opened — the canvas, on every path that gets here. A
    /// handle of our own would leave the window focused on nothing once the panel closes.
    restore_focus: Option<FocusHandle>,
    _document: Subscription,
    _menu: Subscription,
    _query: Subscription,
}

impl std::fmt::Debug for PagesPanel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PagesPanel")
            .field("cursor", &self.cursor)
            .finish_non_exhaustive()
    }
}

impl PagesPanel {
    pub(crate) fn new(
        canvas: Entity<CanvasView>,
        canvas_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search pages…"));
        let typing = cx.subscribe_in(&query, window, |this, _, event, window, cx| match event {
            InputEvent::Change => this.requery(cx),
            InputEvent::PressEnter { .. } => this.confirm(window, cx),
            _ => {}
        });
        // A sibling entity: without this a page added or renamed elsewhere leaves the list stale.
        let document = canvas.read(cx).document().clone();
        let repaint = cx.observe(&document, |_, _, cx| cx.notify());
        // The open flag lives outside this view — the pill and `Page::OpenPicker` both write it
        // — so opening is something to observe rather than a method anyone calls.
        let menu = cx.observe_global_in::<PagesMenu>(window, |this, window, cx| {
            if PagesMenu::is_open(cx) {
                this.opened(window, cx);
            } else {
                this.closed(window, cx);
            }
            cx.notify();
        });

        Self {
            canvas,
            canvas_focus,
            query,
            cursor: 0,
            rows_scroll: ScrollHandle::default(),
            restore_focus: None,
            _document: repaint,
            _menu: menu,
            _query: typing,
        }
    }

    /// Every open starts on the page you are looking at, as the reference's cursor reset does,
    /// and in the search box, so `o` followed by typing is one gesture.
    fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_focus = window.focused(cx);
        self.move_cursor_to(self.active_index(cx));
        let query = self.query.clone();
        query.update(cx, |query, cx| {
            query.set_value("", window, cx);
            query.focus(window, cx);
        });
    }

    fn closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.restore_focus.take() {
            window.focus(&handle, cx);
        }
    }

    /// Re-filtering changes which row the cursor is over, so it goes back to the best match
    /// rather than keeping an index into a list that no longer exists. Clearing the box is not a
    /// search, so it returns the cursor to the page you are on.
    fn requery(&mut self, cx: &mut Context<Self>) {
        let landing = if self.is_searching(cx) {
            0
        } else {
            self.active_index(cx)
        };
        self.move_cursor_to(landing);
        cx.notify();
    }

    fn move_cursor_to(&mut self, index: usize) {
        self.cursor = index;
        self.rows_scroll.scroll_to_item(index);
    }

    fn is_searching(&self, cx: &App) -> bool {
        !self.query.read(cx).value().trim().is_empty()
    }

    fn active_index(&self, cx: &App) -> usize {
        self.rows(cx)
            .iter()
            .position(|row| row.active)
            .unwrap_or_default()
    }

    fn rows(&self, cx: &App) -> Vec<PageRow> {
        let document = self.canvas.read(cx).document().read(cx);
        search::filter(search::rows(document), &self.query.read(cx).value())
    }

    /// Moves the cursor by `step`, clamped rather than wrapping — the reference's
    /// `Math.max(0, …)` / `Math.min(len - 1, …)`.
    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.rows(cx).len();
        if count == 0 {
            return;
        }
        let moved = isize::try_from(self.cursor)
            .unwrap_or(0)
            .saturating_add(step);
        self.move_cursor_to(usize::try_from(moved).unwrap_or(0).min(count - 1));
        cx.notify();
    }

    fn confirm(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows(cx).get(self.cursor).cloned() else {
            return;
        };
        self.choose(&row.id, cx);
    }

    fn choose(&mut self, page: &PageId, cx: &mut Context<Self>) {
        self.canvas
            .update(cx, |canvas, cx| canvas.switch_to_page(page, cx));
        // Closing hands focus back, through the observer that watches the flag.
        PagesMenu::close(cx);
        cx.notify();
    }

    /// `up` / `down` / `escape`, taken on the panel root rather than bound as actions.
    ///
    /// This works because a *single-line* `InputState` registers no `MoveUp`/`MoveDown`
    /// listener — gpui-base gates those on `is_multi_line()` — so the keystroke finds no handler
    /// and propagation reaches this ancestor. Escape arrives the same way: the input's own
    /// handler propagates when there is nothing to clear. Enter is the exception, handled off
    /// `InputEvent::PressEnter`, because the field does claim it.
    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "down" => self.step(1, cx),
            "up" => self.step(-1, cx),
            "escape" => PagesMenu::close(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for PagesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A closed panel still renders, as an empty layer: it is an entity child, so it repaints
        // on its own `notify` without the workspace having to observe the open flag too.
        let layer = div().id("pages-panel-layer").absolute().inset_0();
        if !PagesMenu::is_open(cx) {
            return layer.invisible();
        }
        layer
            .child(scrim(cx))
            .child(self.panel(window, cx))
            .on_key_down(cx.listener(Self::on_key_down))
    }
}

impl PagesPanel {
    fn panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        let rows = self.rows(cx);
        // In fullscreen macOS takes the traffic lights back, so the pill moves left with them.
        let left = if window.is_fullscreen() {
            px(12.0)
        } else {
            super::super::TRAFFIC_LIGHT_INSET
        };

        div()
            .id("pages-list")
            .test_support()
            .absolute()
            .left(left)
            .top(super::super::HEIGHT + GAP_UNDER_PILL)
            .w(WIDTH)
            .max_h(MAX_HEIGHT)
            .occlude()
            .v_flex()
            .overflow_hidden()
            .bg(theme.node_bg)
            .border_1()
            .border_color(theme.node_border)
            .rounded(theme.radius_card)
            .when_some(theme.node_shadow, |this, (offset_y, blur, color)| {
                this.shadow(vec![BoxShadow {
                    color,
                    offset: point(px(0.0), offset_y),
                    blur_radius: blur,
                    spread_radius: px(0.0),
                    inset: false,
                }])
            })
            .child(self.search_row(cx))
            .child(self.list(&rows, cx))
            .child(self.footer(cx))
    }

    /// A magnifier and a borderless field, as the connection picker's search row is.
    fn search_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme();
        div()
            .h_flex()
            .gap(px(8.0))
            .pt(px(10.0))
            .px(px(12.0))
            .pb(px(8.0))
            .child(
                Icon::new(IconName::Search)
                    .size(px(13.0))
                    .text_color(theme.fg_subtle),
            )
            .child(
                div()
                    .id("pages-search")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.query).appearance(false)),
            )
    }

    fn list(&self, rows: &[PageRow], cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme();
        let body = div()
            .id("pages-rows")
            .track_scroll(&self.rows_scroll)
            .v_flex()
            .gap(px(2.0))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(8.0))
            .pb(px(6.0));

        if rows.is_empty() {
            return body.child(
                div()
                    .py(px(14.0))
                    .w_full()
                    .text_center()
                    .text_xs()
                    .text_color(theme.fg_subtle)
                    .child("No pages match."),
            );
        }
        body.children(
            rows.iter()
                .enumerate()
                .map(|(index, row)| self.row(row, index == self.cursor, cx)),
        )
    }

    fn row(&self, row: &PageRow, under_cursor: bool, cx: &mut Context<Self>) -> AnyElement {
        let (fg, fg_muted, fg_subtle, green, radius, lit) = {
            let theme = cx.peek_theme();
            (
                theme.fg,
                theme.fg_muted,
                theme.fg_subtle,
                theme.green,
                theme.radius_card,
                theme.node_bg_2,
            )
        };
        let dot = if row.active { green } else { fg_subtle };
        let id = row.id.clone();

        div()
            .id(SharedString::from(format!("page-row-{}", row.id)))
            .test_support()
            .aria_selected(under_cursor)
            .h_flex()
            .gap(px(8.0))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(radius)
            .when(under_cursor, |this| this.bg(lit))
            .hover(move |style| style.bg(lit))
            .text_xs()
            .text_color(if row.active { fg } else { fg_muted })
            .child(
                div()
                    .size(DOT)
                    .flex_shrink_0()
                    .rounded_full()
                    .when(row.active, |this| this.bg(dot))
                    .when(!row.active, |this| this.border_1().border_color(dot)),
            )
            .child(
                div()
                    .h_flex()
                    .flex_1()
                    .min_w_0()
                    .children(crate::fuzzy::highlight(&row.name, &row.matched, cx)),
            )
            .when(row.closable, |this| {
                this.child(close_button(row, &self.canvas_focus))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.choose(&id, cx)))
            .into_any_element()
    }

    /// Both halves earn their place, as the connection picker's footer does: the left one acts
    /// on the list above it, and the right one names the key that acts on the cursor.
    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme();
        div()
            .h_flex()
            .justify_between()
            .items_center()
            .px(px(8.0))
            .py(px(6.0))
            .border_t_1()
            .border_color(theme.node_border)
            .bg(theme.fg.opacity(0.02))
            .child(add_button(&self.canvas_focus, "New page"))
            .child(
                div()
                    .h_flex()
                    .gap(px(6.0))
                    .pr(px(4.0))
                    .text_size(px(10.5))
                    .font_family("Monaspace Krypton")
                    .text_color(theme.fg_subtle)
                    .child("\u{21b5}")
                    .child("switch"),
            )
    }
}

/// A full-window sibling that swallows the press that dismisses the panel. It paints nothing:
/// the reference dims nothing behind its popover, and `occlude` is a hitbox property rather than
/// a painted one, so a transparent layer still blocks.
fn scrim(cx: &mut Context<PagesPanel>) -> impl IntoElement {
    div()
        .id("pages-panel-scrim")
        .test_support()
        .absolute()
        .inset_0()
        .occlude()
        .bg(transparent_black())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _: &MouseDownEvent, _, cx| PagesMenu::close(cx)),
        )
}
