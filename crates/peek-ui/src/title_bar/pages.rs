//! Page tabs, ported from `~/labs/peek/src/components/titlebar/PageSelector/PageSelector.tsx`.
//!
//! Hand-built pills rather than gpui-component's `TabBar`: its `Tab::render` uses the tab's list
//! index as its `ElementId`, which `CLAUDE.md` forbids and which would make every test id
//! position-dependent. It also has no close affordance and no reorder, and its variant tables
//! set their own height, padding and radius — we would be overriding every visual decision to
//! reuse a click handler. The reference's tabs are pills, not tabs.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::TestSupportExt;
use gpui_kit::base::input::{Input, InputEvent, InputState};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, BoxShadow, ClickEvent, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
    SharedString, Subscription, Window, div, point, px, rems,
};
use peek_document::PageId;
use peek_theme::ActivePeekTheme;

use crate::canvas::CanvasView;
use crate::commands::actions;

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
    _document: Subscription,
}

impl std::fmt::Debug for PageTabs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PageTabs")
            .field("renaming", &self.rename.is_some())
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
        Self {
            canvas,
            canvas_focus,
            rename: None,
            _document: subscription,
        }
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
}
