//! The Text node: `~/labs/peek/src/canvas/nodes/Text/TextNode.tsx`.
//!
//! Unlike every other kind it draws no shell chrome — no header, no indicator, just the text
//! on its own card. Double-click enters edit mode; `Enter` leaves it and deselects.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::TestSupportExt;
use gpui_kit::base::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, Entity, FocusHandle, Focusable, FontWeight, Pixels, SharedString,
    Subscription, WeakEntity, Window, div, px, rems,
};
use peek_canvas::{Document, Size};
use peek_document::{Node, NodeId, TextData};
use peek_theme::ActivePeekTheme;

use super::BASE_REM;
use super::kind::NodeContext;
use super::state::NodeState;

/// `TextNode.tsx`'s constants. They are world units: there the flow is CSS-scaled by the zoom,
/// here the node is laid out in a rem scope of `BASE_REM * zoom`, so the same number means the
/// same thing in both.
const FONT_SIZE_RATIO: f32 = 0.62;
const MIN_FONT_SIZE: f32 = 12.0;
/// `.text-node`'s `line-height: 1.5`.
const LINE_HEIGHT_RATIO: f32 = 1.5;
/// Slack the reference adds on top of the measured text before it grows the node.
const WIDTH_PADDING: f32 = 16.0;
/// `.text-node-display` / `.text-node-input` / `.text-node-measure` all share `padding: 4px 6px`.
const PADDING_X: f32 = 6.0;
const PADDING_Y: f32 = 4.0;
/// The hint `.text-node-placeholder` carries when the node has no text yet.
const PLACEHOLDER: &str = "Double-click to edit";

/// Retained editing state: the editor and the focus the node parks on when the editor closes.
///
/// There is no `editing` flag. Edit mode *is* the input holding focus — the reference keeps its
/// `isEditing` and the textarea's focus in lockstep (double-click focuses, `onBlur` clears), and
/// reading the one source of truth avoids a second one that [`body`] could not write to anyway.
pub(crate) struct TextState {
    input: Entity<InputState>,
    /// Whatever held focus when the editor opened — the canvas, in every path that reaches
    /// here — so `Enter` and `Escape` hand it back. A handle of the node's own would be the
    /// obvious alternative and is wrong: it dies with the node, and deleting a node while it
    /// held focus would leave the window with nothing focused and no route for `cmd-z`.
    restore_focus: Rc<RefCell<Option<FocusHandle>>>,
    document: WeakEntity<Document>,
    _subscription: Subscription,
}

impl std::fmt::Debug for TextState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TextState").finish_non_exhaustive()
    }
}

impl TextState {
    pub(crate) fn new(
        id: &NodeId,
        data: &TextData,
        document: &Entity<Document>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(data.text.clone())
                .placeholder("Type...")
        });
        let restore_focus = Rc::new(RefCell::new(None));
        let weak_document = document.downgrade();
        let subscription = window.subscribe(&input, cx, {
            let document = weak_document.clone();
            let restore_focus: Rc<RefCell<Option<FocusHandle>>> = Rc::clone(&restore_focus);
            let id = id.clone();
            move |input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    write_text(&document, &id, text, cx);
                }
                InputEvent::PressEnter { .. } => {
                    close_editor(&restore_focus, window, cx);
                    deselect_all(&document, cx);
                }
                InputEvent::Focus | InputEvent::Blur => {}
            }
        });

        // `useState(data.text.length === 0)` plus the autofocus effect: a node placed empty is
        // immediately typable.
        if data.text.is_empty() {
            open_editor(&input, &restore_focus, window, cx);
        }

        Self {
            input,
            restore_focus,
            document: weak_document,
            _subscription: subscription,
        }
    }

    fn render(
        &self,
        id: &NodeId,
        data: &TextData,
        size: Size,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let font = font_size(size);
        let required = required_width(&data.text, font, window);
        if required > world(size.width) {
            grow(
                &self.document,
                id,
                Size::new(f64::from(required), size.height),
                window,
                cx,
            );
        }

        let editing = self.input.focus_handle(cx).is_focused(window);
        let child = if editing {
            Input::new(&self.input).into_any_element()
        } else {
            display(data, cx).into_any_element()
        };

        // Deliberately not focusable itself: the nearest focusable ancestor is the canvas, so
        // a plain click on a text node lands focus there like a click on any other node.
        card(size, cx)
            .id(SharedString::from(format!("{id}-text")))
            .aria_label(SharedString::from(label(data)))
            .test_support()
            .key_context("TextNode")
            // `Tool::Select` is `escape`, and it is bound even while typing. The card sits on
            // the focus path below the canvas, so this runs first and spends the keystroke on
            // closing the editor; a second `escape` then reaches the canvas and clears the
            // selection. A plain `on_key_down` cannot do this: gpui dispatches bound actions
            // before any key listener.
            .on_action({
                let restore_focus = Rc::clone(&self.restore_focus);
                move |_: &crate::commands::actions::tool::Select, window, cx| {
                    close_editor(&restore_focus, window, cx);
                    cx.stop_propagation();
                }
            })
            .on_click({
                let input = self.input.clone();
                let restore_focus = Rc::clone(&self.restore_focus);
                let text = data.text.clone();
                move |event, window, cx| {
                    if event.click_count() < 2 {
                        return;
                    }
                    // Adopt whatever the document holds now, the way `useSyncedFieldValue`
                    // re-adopts its source after an undo or a remote edit.
                    input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                    open_editor(&input, &restore_focus, window, cx);
                }
            })
            .child(child)
            .into_any_element()
    }
}

pub(crate) fn title(data: &TextData) -> String {
    super::placeholder::first_line(&data.text)
}

pub(crate) fn body(
    id: &NodeId,
    data: &TextData,
    context: NodeContext<'_>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let size = context
        .document
        .read(cx)
        .node(id)
        .map(Node::size)
        .unwrap_or_default();
    match context.state {
        Some(NodeState::Text(state)) => state.render(id, data, size, window, cx),
        _ => card(size, cx).child(display(data, cx)).into_any_element(),
    }
}

/// World units are `f64` and gpui's are `f32`. `Pixels` converts from both, so the narrowing
/// happens inside gpui rather than behind a `cast_possible_truncation` allow of our own.
fn world(value: f64) -> f32 {
    f32::from(Pixels::from(value))
}

/// The reference derives the size from the rendered card's height through a `ResizeObserver`;
/// the card is the node, so reading the node's height is the same measurement without the
/// round trip.
fn font_size(size: Size) -> f32 {
    (world(size.height) * FONT_SIZE_RATIO).max(MIN_FONT_SIZE)
}

/// `.text-node`: transparent, no border — the selection ring is painted by the canvas overlay.
///
/// The font size is a *world* length derived from the node's height, so it is spelled in rems:
/// the canvas lays every node out inside a rem scope of `BASE_REM * zoom` at a screen rect of
/// `size * zoom`, which makes `rems(length / BASE_REM)` the one unit that tracks the camera.
/// `px` would pin the text to a fixed screen size and desynchronise it from its own card. The
/// 12-unit floor is a world floor for the same reason: the reference clamps against the
/// unscaled height, before the viewport transform.
fn card(size: Size, cx: &App) -> Div {
    let font = font_size(size);
    div()
        .size_full()
        .overflow_hidden()
        .px(rems(PADDING_X / BASE_REM))
        .py(rems(PADDING_Y / BASE_REM))
        .text_size(rems(font / BASE_REM))
        .line_height(rems(font * LINE_HEIGHT_RATIO / BASE_REM))
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.peek_theme().fg)
}

/// What the card reads as: the node's own words, or the hint standing in for them.
fn label(data: &TextData) -> String {
    if data.text.is_empty() {
        return PLACEHOLDER.to_string();
    }
    data.text.clone()
}

/// `.text-node-display`, which is `white-space: pre`: the node's own line breaks are kept and
/// nothing wraps to fit the card.
fn display(data: &TextData, cx: &App) -> Div {
    let theme = cx.peek_theme();
    if data.text.is_empty() {
        return div()
            .italic()
            .font_weight(FontWeight::NORMAL)
            .text_color(theme.fg_subtle)
            .child(PLACEHOLDER);
    }
    div()
        .whitespace_nowrap()
        .child(SharedString::from(data.text.clone()))
}

/// `.text-node-measure`: the widest line plus the padding the hidden copy carries in its
/// `offsetWidth`, plus the reference's own slack.
///
/// Shaped at the unzoomed font size, so the width that ends up in the document is a world
/// length and does not change with the camera.
fn required_width(text: &str, font: f32, window: &mut Window) -> f32 {
    let mut run = window.text_style().to_run(0);
    run.font.weight = FontWeight::MEDIUM;
    let mut widest = 0.0_f32;
    for line in text.split('\n') {
        run.len = line.len();
        let layout =
            window
                .text_system()
                .layout_line(line, px(font), std::slice::from_ref(&run), None);
        widest = widest.max(f32::from(layout.width));
    }
    widest.ceil() + PADDING_X * 2.0 + WIDTH_PADDING
}

/// The node only ever grows, so re-measuring the wider node settles on the first frame.
/// Deferred because this runs while the canvas is building its element tree.
///
/// A content-derived width, so it goes through `set_intrinsic_size` and opens no undo
/// transaction: it folds into the typing that caused it, and one `cmd-z` takes back both.
fn grow(document: &WeakEntity<Document>, id: &NodeId, size: Size, window: &Window, cx: &mut App) {
    let Some(document) = document.upgrade() else {
        return;
    };
    let id = id.clone();
    window.defer(cx, move |_, cx| {
        document.update(cx, |document, cx| {
            // Revalidated: the node may have been deleted, undone or already widened between
            // the frame that measured it and this callback.
            if document
                .node(&id)
                .is_none_or(|node| node.size().width >= size.width)
            {
                return;
            }
            document.set_intrinsic_size(&id, size);
            cx.notify();
        });
    });
}

fn open_editor(
    input: &Entity<InputState>,
    restore_focus: &RefCell<Option<FocusHandle>>,
    window: &mut Window,
    cx: &mut App,
) {
    *restore_focus.borrow_mut() = window.focused(cx);
    input.update(cx, |input, cx| input.focus(window, cx));
}

fn close_editor(restore_focus: &RefCell<Option<FocusHandle>>, window: &mut Window, cx: &mut App) {
    let Some(handle) = restore_focus.borrow_mut().take() else {
        return;
    };
    window.focus(&handle, cx);
}

fn write_text(document: &WeakEntity<Document>, id: &NodeId, text: String, cx: &mut App) {
    let Some(document) = document.upgrade() else {
        return;
    };
    document.update(cx, |document, cx| {
        if document.update_data::<TextData>(id, |data| data.text = text) {
            cx.notify();
        }
    });
}

fn deselect_all(document: &WeakEntity<Document>, cx: &mut App) {
    let Some(document) = document.upgrade() else {
        return;
    };
    document.update(cx, |document, cx| {
        if document.deselect_all() {
            cx.notify();
        }
    });
}
