//! The close-page confirmation, ported from `~/labs/peek/src/canvas/ClosePageConfirmModal.tsx`.
//!
//! Page deletion is **not undoable** — `Document::delete_page` records no history, matching the
//! TypeScript app — and every other decision here follows from that. A page with no nodes is
//! deleted without asking because there is nothing to lose; anything else asks first, on the
//! keyboard path as well as the button. There is deliberately no optimistic delete with an
//! "Undo" notification: that pattern belongs to reversible destruction.
//!
//! The wording diverges from the reference, which titles this "Close page?" for an operation
//! that permanently discards work. Naming the page and the count, and saying plainly that it
//! cannot be undone, is the honest version.

use gpui_kit::TestSupportExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Sizable, StyledExt, WindowExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, KeyDownEvent, SharedString, WeakEntity, Window,
    div, px, rems,
};
use peek_document::PageId;

use crate::canvas::CanvasView;

pub(crate) struct ClosePageConfirm {
    canvas: WeakEntity<CanvasView>,
    page: PageId,
    name: SharedString,
    nodes: usize,
    focus: FocusHandle,
}

impl std::fmt::Debug for ClosePageConfirm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClosePageConfirm")
            .field("page", &self.page)
            .field("nodes", &self.nodes)
            .finish_non_exhaustive()
    }
}

impl ClosePageConfirm {
    /// Opens the dialog over the window. An entity rather than a bare `Dialog` because the
    /// `y`/`n` accelerators need a focus handle to hang a key listener on.
    pub(crate) fn open(
        canvas: &Entity<CanvasView>,
        page: &PageId,
        name: SharedString,
        nodes: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let confirm = cx.new(|cx| Self {
            canvas: canvas.downgrade(),
            page: page.clone(),
            name,
            nodes,
            focus: cx.focus_handle(),
        });
        let focus = confirm.read(cx).focus.clone();

        window.open_dialog(cx, move |dialog, _, _| {
            let confirm = confirm.clone();
            dialog
                .w(px(400.0))
                .overlay_closable(true)
                .content(move |content, _, _| content.child(confirm.clone()))
        });
        window.focus(&focus, cx);
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Close first, then mutate, so the deletion lands with the dialog's focus released —
        // the ordering the command palette uses.
        window.close_dialog(cx);
        let page = self.page.clone();
        self.canvas
            .update(cx, |canvas, cx| canvas.delete_page(&page, cx))
            .ok();
    }

    fn cancel(window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "y" => self.confirm(window, cx),
            "n" => Self::cancel(window, cx),
            _ => {}
        }
    }
}

impl Focusable for ClosePageConfirm {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ClosePageConfirm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nodes = self.nodes;
        let plural = if nodes == 1 { "node" } else { "nodes" };

        div()
            .id("close-page-confirm")
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .v_flex()
            .gap_4()
            .child(
                div()
                    .text_lg()
                    .font_semibold()
                    .text_color(cx.theme().foreground)
                    .child(format!("Delete \u{201c}{}\u{201d}?", self.name)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{nodes} {plural} on this page will be deleted. This cannot be undone."
                    )),
            )
            .child(
                div()
                    .h_flex()
                    .justify_end()
                    .gap_2()
                    .pt(rems(0.25))
                    .child(
                        Button::new("close-page-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| Self::cancel(window, cx)),
                    )
                    .child(
                        Button::new("close-page-delete")
                            .small()
                            .danger()
                            .label("Delete page")
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    ),
            )
    }
}
