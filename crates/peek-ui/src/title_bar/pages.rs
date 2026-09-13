//! Page tabs, ported from `~/labs/peek/src/components/titlebar/PageSelector/PageSelector.tsx`.
//!
//! Hand-built pills rather than gpui-component's `TabBar`: its `Tab::render` uses the tab's list
//! index as its `ElementId`, which `CLAUDE.md` forbids and which would make every test id
//! position-dependent. It also has no close affordance and no reorder, and its variant tables
//! set their own height, padding and radius — we would be overriding every visual decision to
//! reuse a click handler. The reference's tabs are pills, not tabs.
//!
//! `ui.pages.show_as` picks between the strip and a single pill that opens the page list
//! (`PagesMenu.tsx`). `Settings::TogglePageDisplay` flips it, and this view observes the
//! settings global so the bar follows without `WorkspaceView` knowing anything about it.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{Input, InputEvent, InputState};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Icon, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, BoxShadow, ClickEvent, Context, Entity, FocusHandle, FontWeight, Global,
    KeyDownEvent, SharedString, Subscription, Window, div, point, px, rems,
};
use peek_config::PageDisplay;
use peek_document::PageId;
use peek_theme::ActivePeekTheme;

use crate::canvas::CanvasView;
use crate::commands::actions;
use crate::settings::Settings;

/// Whether the pages picker is open.
///
/// A global rather than a field on [`PageTabs`] because the two things that open it are in
/// different subtrees: the pill lives in the title bar, while `Page::OpenPicker` is dispatched
/// through the canvas focus handle and never reaches it. The reference shares a jotai atom
/// (`pagesMenuOpenAtom`) between the same two callers for the same reason.
#[derive(Debug, Default)]
pub(crate) struct PagesMenu {
    open: bool,
}

impl Global for PagesMenu {}

impl PagesMenu {
    pub(crate) fn is_open(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|menu| menu.open)
    }

    pub(crate) fn toggle(cx: &mut App) {
        Self::set(!Self::is_open(cx), cx);
    }

    pub(crate) fn close(cx: &mut App) {
        Self::set(false, cx);
    }

    /// Guarded because every write notifies observers, and the popover writes back its own
    /// state on open and close — an unguarded set would repaint the bar for no change.
    fn set(open: bool, cx: &mut App) {
        if Self::is_open(cx) == open {
            return;
        }
        cx.update_default_global::<Self, ()>(|menu, _| menu.open = open);
    }
}

/// One frame's view of a page, snapshotted so the document borrow ends before the listeners
/// that need `&mut Context` are built.
struct PageRow {
    id: PageId,
    name: SharedString,
    active: bool,
    closable: bool,
}

/// A live inline rename. Dropping it cancels: the subscription dies with it, so the `Blur`
/// that follows moving focus away reaches no handler.
struct Rename {
    page: PageId,
    /// What to restore when the field is committed empty, as the reference's
    /// `e.target.value || page.name`.
    original: SharedString,
    input: Entity<InputState>,
    /// Whatever held focus when the editor opened — the canvas, on every path that gets here.
    /// A handle of our own would die with the tab and leave the window focused on nothing.
    restore_focus: Rc<RefCell<Option<FocusHandle>>>,
    _subscription: Subscription,
}

pub(crate) struct PageTabs {
    canvas: Entity<CanvasView>,
    /// Chrome dispatches through the canvas, so buttons take the same path the keyboard does.
    canvas_focus: FocusHandle,
    rename: Option<Rename>,
    /// The picker's own handle, handed to the popover so the list is what the arrow keys reach.
    /// Without it the popover focuses a handle of its own and every key listener below sits off
    /// the focus path, which dispatch never walks into.
    picker_focus: FocusHandle,
    /// Which row the picker's arrow keys are on.
    picker_cursor: usize,
    _document: Subscription,
    _globals: [Subscription; 2],
}

impl std::fmt::Debug for PageTabs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PageTabs")
            .field("renaming", &self.rename.is_some())
            .field("picker_cursor", &self.picker_cursor)
            .finish_non_exhaustive()
    }
}

impl PageTabs {
    pub(crate) fn new(
        canvas: Entity<CanvasView>,
        canvas_focus: FocusHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        // A sibling entity: without this a document change re-renders the canvas and leaves the
        // tabs showing a stale page list.
        let document = canvas.read(cx).document().clone();
        let subscription = cx.observe(&document, |_, _, cx| cx.notify());
        // The display mode and the picker's open state both live outside this view, so both
        // need an observer: nothing else in the title bar would repaint it.
        let settings = cx.observe_global::<Settings>(|_, cx| cx.notify());
        let picker = cx.observe_global::<PagesMenu>(|this, cx| {
            // Every open starts on the page you are looking at, as the reference's cursor reset
            // does — whether the popover was clicked or `o` was pressed.
            if PagesMenu::is_open(cx) {
                this.picker_cursor = this.active_index(cx);
            }
            cx.notify();
        });
        Self {
            canvas,
            canvas_focus,
            rename: None,
            picker_focus: cx.focus_handle(),
            picker_cursor: 0,
            _document: subscription,
            _globals: [settings, picker],
        }
    }

    fn active_index(&self, cx: &App) -> usize {
        let document = self.canvas.read(cx).document().read(cx);
        let active = document.active_page_id();
        document
            .pages()
            .position(|page| &page.id == active)
            .unwrap_or_default()
    }

    fn rows(&self, cx: &App) -> Vec<PageRow> {
        let document = self.canvas.read(cx).document().read(cx);
        let active = document.active_page_id().clone();
        // The `×` needs somewhere to go: the last page cannot be deleted.
        let closable = document.page_count() > 1;
        document
            .pages()
            .map(|page| PageRow {
                id: page.id.clone(),
                name: SharedString::from(page.name.clone()),
                active: page.id == active,
                closable: closable && page.id == active,
            })
            .collect()
    }

    fn begin_rename(
        &mut self,
        page: &PageId,
        name: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
        // `onFocus={e => e.currentTarget.select()}`: typing replaces rather than appends.
        input.update(cx, |input, cx| input.select_all(window, cx));

        let restore_focus = Rc::new(RefCell::new(window.focused(cx)));
        let subscription =
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                // The reference commits on both, and there is no per-keystroke write: renaming
                // records no undo step, so a field with a natural commit point should use it.
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_rename(window, cx);
                }
            });
        input.update(cx, |input, cx| input.focus(window, cx));

        self.rename = Some(Rename {
            page: page.clone(),
            original: name.clone(),
            input,
            restore_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        let typed = rename.input.read(cx).value().trim().to_string();
        let name = if typed.is_empty() {
            rename.original.to_string()
        } else {
            typed
        };
        let page = rename.page.clone();
        let restore = rename.restore_focus.borrow().clone();
        // Drop before moving focus: the resulting `Blur` must not reach a live subscription.
        drop(rename);

        self.canvas
            .update(cx, |canvas, cx| canvas.rename_page(&page, name, cx));
        if let Some(handle) = restore {
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        let restore = rename.restore_focus.borrow().clone();
        drop(rename);
        if let Some(handle) = restore {
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // `Tool::Select` owns escape, but it is bound on `"Canvas"` and the title bar is not
        // under that context. The base input binds escape on `"Input"` and its handler
        // propagates when there is nothing to clear, so this root — above it on the focus path
        // — still sees the keystroke.
        if self.rename.is_some() && event.keystroke.key == "escape" {
            self.cancel_rename(window, cx);
            cx.stop_propagation();
        }
    }

    fn tab(&self, row: &PageRow, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme();
        let (fg, dot) = if row.active {
            (theme.fg, theme.green)
        } else {
            (theme.fg_muted, theme.fg_subtle)
        };
        let lit_text = theme.fg;
        let lit_surface = theme.node_bg_2;
        let radius = theme.radius_pill;
        let border = theme.node_border;
        let glow = theme.green;

        let id = row.id.clone();
        let name = row.name.clone();

        div()
            .id(SharedString::from(format!("page-tab-{}", row.id)))
            .test_support()
            .h_flex()
            .gap(rems(0.375))
            .px_3()
            .py_1()
            .max_w(rems(15.0))
            .flex_shrink_0()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .when(row.active, |this| this.bg(lit_surface))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg)
            .hover(move |style| style.bg(lit_surface).text_color(lit_text))
            .child(
                div()
                    .size(rems(0.375))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(dot)
                    .when(row.active, move |this| {
                        this.shadow(vec![BoxShadow {
                            color: glow,
                            offset: point(px(0.0), px(0.0)),
                            blur_radius: px(8.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                    }),
            )
            .child(div().min_w_0().truncate().child(name.clone()))
            .when(row.closable, |this| {
                this.child(close_button(row, &self.canvas_focus))
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                // One handler for both gestures, as the DOM does. Switching an already-active
                // page is a no-op inside `Document::switch_page`, so double-clicking the active
                // tab renames without a stray switch.
                if event.click_count() >= 2 {
                    this.begin_rename(&id, &name, window, cx);
                    return;
                }
                this.canvas
                    .update(cx, |canvas, cx| canvas.switch_to_page(&id, cx));
            }))
    }

    fn rename_field(rename: &Rename) -> impl IntoElement {
        div()
            .id("page-rename")
            .test_support()
            .px_3()
            .py_1()
            .max_w(rems(15.0))
            .flex_shrink_0()
            .child(Input::new(&rename.input))
    }
}

/// Only on the active tab, and only when another page exists to fall back to — not hover-gated,
/// so the state stays visible.
fn close_button(row: &PageRow, canvas_focus: &FocusHandle) -> impl IntoElement {
    let focus = canvas_focus.clone();
    Button::new(SharedString::from(format!("page-close-{}", row.id)))
        .ghost()
        .xsmall()
        .label("\u{00d7}")
        .tooltip_with_action(
            "Delete page",
            &actions::page::Close,
            Some(crate::commands::CANVAS),
        )
        .on_click(move |_, window, cx| {
            // The pill's own click sits underneath and would switch pages.
            cx.stop_propagation();
            focus.dispatch_action(&actions::page::Close, window, cx);
        })
}

fn add_button(canvas_focus: &FocusHandle) -> impl IntoElement {
    let focus = canvas_focus.clone();
    Button::new("page-add")
        .ghost()
        .xsmall()
        .label("+")
        .tooltip_with_action(
            "New page",
            &actions::page::New,
            Some(crate::commands::CANVAS),
        )
        .on_click(move |_, window, cx| {
            focus.dispatch_action(&actions::page::New, window, cx);
        })
}

impl Render for PageTabs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Copied out: the rest of this frame needs `cx` mutably, and the config borrow holds it.
        let display = Settings::get(cx).ui.pages.show_as;
        match display {
            PageDisplay::Tabs => self.tab_strip(cx).into_any_element(),
            PageDisplay::List => self.picker(window, cx).into_any_element(),
        }
    }
}

impl PageTabs {
    fn tab_strip(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let renaming = self.rename.as_ref().map(|rename| rename.page.clone());

        div()
            .id("page-tabs")
            .test_support()
            .on_key_down(cx.listener(Self::on_key_down))
            .h_flex()
            .gap_1()
            .min_w_0()
            .overflow_hidden()
            .children(rows.iter().map(|row| {
                if Some(&row.id) == renaming.as_ref()
                    && let Some(rename) = self.rename.as_ref()
                {
                    return Self::rename_field(rename).into_any_element();
                }
                self.tab(row, cx).into_any_element()
            }))
            .child(add_button(&self.canvas_focus))
    }

    /// List mode: one pill naming the active page, and the page list behind it.
    fn picker(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let label = rows
            .iter()
            .find(|row| row.active)
            .map_or_else(|| SharedString::new_static("Pages"), |row| row.name.clone());
        let badge =
            Kbd::binding_for_action_in(&actions::page::OpenPicker, &self.canvas_focus, window);
        let list = cx.entity();

        div().id("page-picker").test_support().child(
            Popover::new("pages-menu")
                .open(PagesMenu::is_open(cx))
                // Controlled, so the click on the pill and the `o` key write to the one place
                // both read from.
                .on_open_change(|open, _, cx| PagesMenu::set(*open, cx))
                .track_focus(&self.picker_focus)
                .trigger(pill(label, badge, cx))
                .content(move |_, _, cx| {
                    list.update(cx, |this, cx| this.page_list(cx).into_any_element())
                }),
        )
    }

    fn page_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let cursor = self.picker_cursor.min(rows.len().saturating_sub(1));
        let theme = cx.peek_theme();

        div()
            .id("pages-list")
            .test_support()
            .track_focus(&self.picker_focus)
            .on_key_down(cx.listener(Self::on_picker_key))
            .v_flex()
            .gap(px(2.0))
            .min_w(rems(13.0))
            .text_xs()
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .items_center()
                    .pb_1()
                    .text_color(theme.fg_subtle)
                    .child("Pages")
                    .child(add_button(&self.canvas_focus)),
            )
            .children(
                rows.iter()
                    .enumerate()
                    .map(|(index, row)| self.page_row(row, index == cursor, cx)),
            )
    }

    fn page_row(&self, row: &PageRow, under_cursor: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme();
        let dot_color = if row.active {
            theme.green
        } else {
            theme.fg_subtle
        };
        let id = row.id.clone();

        div()
            .id(SharedString::from(format!("page-row-{}", row.id)))
            .test_support()
            .h_flex()
            .gap(px(8.0))
            .px_2()
            .py_1()
            .rounded(theme.radius_card)
            .when(under_cursor, |this| this.bg(theme.node_bg_2))
            .text_color(if row.active { theme.fg } else { theme.fg_muted })
            .child(
                div()
                    .size(rems(0.375))
                    .flex_shrink_0()
                    .rounded_full()
                    .when(row.active, |this| this.bg(dot_color))
                    .when(!row.active, |this| this.border_1().border_color(dot_color)),
            )
            .child(div().flex_1().min_w_0().truncate().child(row.name.clone()))
            .when(row.closable, |this| {
                this.child(close_button(row, &self.canvas_focus))
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.choose_page(&id, cx);
            }))
            .into_any_element()
    }

    /// The reference's list hotkeys: the arrows walk it, Enter takes the row. Escape belongs to
    /// the popover, which binds `Cancel` on the context it owns.
    fn on_picker_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let last = self
            .canvas
            .read(cx)
            .document()
            .read(cx)
            .page_count()
            .saturating_sub(1);
        match event.keystroke.key.as_str() {
            "up" => self.picker_cursor = self.picker_cursor.saturating_sub(1),
            "down" => self.picker_cursor = (self.picker_cursor + 1).min(last),
            "enter" => {
                if let Some(id) = self.page_at_cursor(cx) {
                    self.choose_page(&id, cx);
                }
            }
            _ => return,
        }
        // Enter is also the popover's own `Confirm`, which toggles it: without this the row is
        // chosen, the menu closes, and then the popover opens it straight back up.
        cx.stop_propagation();
        cx.notify();
    }

    fn page_at_cursor(&self, cx: &App) -> Option<PageId> {
        self.canvas
            .read(cx)
            .document()
            .read(cx)
            .pages()
            .nth(self.picker_cursor)
            .map(|page| page.id.clone())
    }

    fn choose_page(&mut self, id: &PageId, cx: &mut Context<Self>) {
        self.canvas
            .update(cx, |canvas, cx| canvas.switch_to_page(id, cx));
        PagesMenu::close(cx);
        cx.notify();
    }
}

/// `.pages-pill`: the list-mode trigger. As with the connection pill the surface sits on an
/// inner element, because `Button::render` already sets a hover style and gpui allows one.
fn pill(label: SharedString, badge: Option<Kbd>, cx: &App) -> Button {
    let theme = cx.peek_theme();
    Button::new("pages-pill")
        .ghost()
        .xsmall()
        .p_0()
        .child(
            div()
                .h_flex()
                .gap(px(6.0))
                .px_3()
                .py_1()
                .max_w(rems(15.0))
                .rounded(theme.radius_pill)
                .border_1()
                .border_color(theme.node_border)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.fg)
                .child(
                    div()
                        .size(rems(0.375))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(theme.green),
                )
                .child(div().min_w_0().truncate().child(label))
                .child(Icon::new(IconName::ChevronDown).size(px(8.0)))
                .children(badge),
        )
        .tooltip("Pages")
}
