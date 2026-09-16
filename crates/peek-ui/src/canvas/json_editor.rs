//! The JSON editor, drawn beside the cell it belongs to.
//!
//! **Chrome, not content**, and for the opposite reason to the value pane
//! (`node/result/detail.rs`, which argues itself into the node). A pane *explaining* a cell is
//! content and should grow with the camera; a field you *type into* should not — at zoom 0.4 a
//! scaled editor renders four-pixel text, and the document you are editing is the one thing on
//! screen that has to stay legible. So this is an absolutely positioned sibling of
//! `CanvasElement` at rem 1, like `context_menu.rs`, and it follows the cell rather than
//! belonging to it.
//!
//! The panel holds no state of its own: the draft, the validity and the failure all live on the
//! `ResultTable`, which owns the commit path. What is here is where to draw it.

use gpui_kit::TestSupportExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Editor;
use gpui_kit::component::{Disableable, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, BoxShadow, Context, MouseButton, MouseDownEvent, Pixels, SharedString,
    WeakEntity, div, point, px,
};
use peek_theme::ActivePeekTheme;

use super::CanvasView;
use crate::node::result::ResultTable;

/// The panel's width. Wider than the context menu: this holds a document, not a list of verbs.
const WIDTH: f32 = 380.0;
/// How tall the editor itself gets. `MonacoJsonCell` grows with its content between 60 and 320;
/// a fixed height costs nothing here because the editor scrolls, and a panel that resized as
/// you typed would walk away from the cell it is anchored to.
const EDITOR_HEIGHT: f32 = 220.0;
const FOOTER_HEIGHT: f32 = 30.0;
const HEADER_HEIGHT: f32 = 24.0;
const PADDING: f32 = 6.0;
/// The gap between the cell and the panel, so the anchor stays visible under it.
const OFFSET: f32 = 4.0;
/// Kept clear of the pane's edges, so a panel opened in the corner is still whole.
const EDGE_MARGIN: f32 = 8.0;

/// Where the panel is, and whose cell it belongs to.
pub(crate) struct JsonEditorState {
    /// The table that owns the draft and the commit. Weak: the canvas outlives the node, and a
    /// node deleted under an open editor must not keep it alive.
    pub(crate) table: WeakEntity<ResultTable>,
    pub(crate) column: SharedString,
    /// The anchor cell in pane coordinates, refreshed every frame the cell is drawn — so the
    /// panel tracks it through a pan, a zoom or a scroll of the rows.
    pub(crate) anchor: Bounds<Pixels>,
}

impl std::fmt::Debug for JsonEditorState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JsonEditorState")
            .field("column", &self.column)
            .field("anchor", &self.anchor)
            .finish_non_exhaustive()
    }
}

impl JsonEditorState {
    fn height() -> f32 {
        HEADER_HEIGHT + EDITOR_HEIGHT + FOOTER_HEIGHT + PADDING * 2.0
    }

    /// Where the panel's top-left goes, given the pane it has to stay inside.
    ///
    /// Below the cell and aligned to its left edge by preference; above it when there is no room
    /// below; and never past an edge. The same rule `context_menu.rs` follows, anchored to a
    /// rectangle rather than to a point.
    fn origin(&self, pane: (f32, f32)) -> (f32, f32) {
        let (pane_width, pane_height) = pane;
        let height = Self::height();
        let left = f32::from(self.anchor.origin.x);
        let top = f32::from(self.anchor.origin.y);
        let bottom = top + f32::from(self.anchor.size.height);

        let left = left.min(pane_width - WIDTH - EDGE_MARGIN).max(EDGE_MARGIN);
        let below = bottom + OFFSET;
        let top = if below + height + EDGE_MARGIN > pane_height {
            (top - height - OFFSET).max(EDGE_MARGIN)
        } else {
            below
        };
        (left, top)
    }
}

pub(super) fn render(view: &CanvasView, cx: &mut Context<CanvasView>) -> Option<AnyElement> {
    let editor = view.json_editor.as_ref()?;
    let table = editor.table.upgrade()?;
    let (left, top) = editor.origin(view.pane_size_for_menu());
    let theme = cx.peek_theme().clone();
    let (saving, error) = table.read(cx).json_edit_status(cx);
    let valid = table.read(cx).json_draft_is_valid(cx);
    let field = table.read(cx).json_editor_state().clone();

    Some(
        div()
            .absolute()
            .inset_0()
            // A press anywhere else closes it, the same dismissal `context_menu.rs` uses. An
            // edit in flight is left alone: cancelling it would not stop the statement.
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    view.close_json_editor(cx);
                }),
            )
            .child(
                div()
                    .id("json-editor")
                    .test_support()
                    // The scrim dismisses on mouse *down*, so without the panel claiming its own
                    // presses it would be gone before any button inside it completed its click.
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx: &mut App| {
                        cx.stop_propagation();
                    })
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(WIDTH))
                    .p(px(PADDING))
                    .v_flex()
                    .rounded(theme.radius_card)
                    .border_1()
                    .border_color(theme.node_border)
                    .bg(theme.node_bg)
                    .when_some(theme.node_shadow, |panel, (offset_y, blur, color)| {
                        panel.shadow(vec![BoxShadow {
                            color,
                            offset: point(px(0.0), offset_y),
                            blur_radius: blur,
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                    })
                    .child(header(&editor.column, cx))
                    .child(
                        div()
                            .h(px(EDITOR_HEIGHT))
                            .w_full()
                            // `appearance(false)` drops the editor's own frame and with it the
                            // inset it drew, so the text would otherwise start flush against
                            // the panel's border.
                            .px(px(4.0))
                            .child(
                                Editor::new(&field)
                                    // Without an explicit size the editor lays out `h_auto` and
                                    // draws only the rows its intrinsic height fits — which here
                                    // was two, in a panel with room for a dozen. The query node
                                    // hit the same thing (`node/query/mod.rs`).
                                    .size_full()
                                    // The panel already draws the frame and the ground; the
                                    // editor's own would sit a second border inside the first.
                                    .appearance(false)
                                    .bordered(false)
                                    // Pixels, not rems: the panel is chrome pinned at rem 1, so
                                    // there is no camera scale for this to track.
                                    .text_size(px(12.0)),
                            ),
                    )
                    .child(footer((valid, saving), error, &editor.table, cx)),
            )
            .into_any_element(),
    )
}

fn header(column: &SharedString, cx: &App) -> impl IntoElement {
    div()
        .h(px(HEADER_HEIGHT))
        .flex_none()
        .h_flex()
        .items_center()
        .px(px(4.0))
        .text_xs()
        .text_color(cx.peek_theme().fg_subtle)
        .child(column.clone())
}

/// `MonacoJsonCell`'s footer: whether the draft parses, and the three things you can do with it.
fn footer(
    validity: (bool, bool),
    error: Option<SharedString>,
    table: &WeakEntity<ResultTable>,
    cx: &App,
) -> impl IntoElement {
    let (valid, saving) = validity;
    let theme = cx.peek_theme();
    let (dot, label) = if valid {
        (theme.green, "valid json")
    } else {
        (theme.red, "invalid json")
    };
    let format = table.clone();
    let save = table.clone();

    div()
        .flex_none()
        .v_flex()
        .children(error.map(|message| {
            div()
                .w_full()
                .px(px(4.0))
                .py(px(2.0))
                .text_size(px(10.0))
                .text_color(theme.red)
                .child(message)
        }))
        .child(
            div()
                .h(px(FOOTER_HEIGHT))
                .h_flex()
                .items_center()
                .gap(px(6.0))
                .px(px(4.0))
                .text_size(px(10.0))
                .text_color(theme.fg_subtle)
                .child(div().flex_none().text_color(dot).child("●"))
                .child(div().flex_none().child(label))
                .child(div().flex_1())
                // No ⌘S, unlike `MonacoJsonCell`: `commands/keymap.rs` resolves one action per
                // keystroke regardless of context — the shape `settings.json`'s `keymap` has and
                // cannot change — and ⌘S already means `Query::Format`. A footer promising a key
                // that silently stole another command's would be worse than the button.
                .child(div().flex_none().child("Esc"))
                .child(
                    Button::new("json-editor-format")
                        .ghost()
                        .xsmall()
                        .label("Format")
                        .disabled(saving || !valid)
                        .on_click(move |_, window, cx| {
                            format
                                .update(cx, |table, cx| table.format_json_draft(window, cx))
                                .ok();
                        }),
                )
                .child(
                    Button::new("json-editor-save")
                        .primary()
                        .xsmall()
                        .label("Save")
                        .disabled(saving || !valid)
                        .on_click(move |_, _, cx| {
                            save.update(cx, ResultTable::commit_edit).ok();
                        }),
                ),
        )
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Bounds, WeakEntity, point, px, size};

    use super::{EDGE_MARGIN, JsonEditorState, OFFSET, WIDTH};

    fn anchored(x: f32, y: f32) -> JsonEditorState {
        JsonEditorState {
            table: WeakEntity::new_invalid(),
            column: "payload".into(),
            anchor: Bounds {
                origin: point(px(x), px(y)),
                size: size(px(180.0), px(34.0)),
            },
        }
    }

    #[test]
    fn the_panel_sits_under_the_cell_and_lines_up_with_it() {
        let (left, top) = anchored(200.0, 100.0).origin((1200.0, 800.0));
        assert!((left - 200.0).abs() < f32::EPSILON, "{left}");
        assert!((top - (134.0 + OFFSET)).abs() < f32::EPSILON, "{top}");
    }

    /// A cell near the bottom would otherwise open a panel that runs off the pane.
    #[test]
    fn a_cell_low_in_the_pane_opens_the_panel_above_it() {
        let (_, top) = anchored(200.0, 700.0).origin((1200.0, 800.0));
        assert!(top < 700.0, "it flipped above the cell: {top}");
        assert!(top >= EDGE_MARGIN, "and still inside the pane: {top}");
    }

    #[test]
    fn a_cell_near_the_right_edge_pulls_the_panel_back_inside() {
        let (left, _) = anchored(1150.0, 100.0).origin((1200.0, 800.0));
        assert!(
            left + WIDTH <= 1200.0 - EDGE_MARGIN + f32::EPSILON,
            "{left}"
        );
    }

    /// A pane narrower than the panel must still produce a left edge on screen rather than a
    /// negative one, which would put the whole thing out of reach.
    #[test]
    fn a_pane_narrower_than_the_panel_still_starts_on_screen() {
        let (left, _) = anchored(10.0, 100.0).origin((200.0, 800.0));
        assert!(left >= EDGE_MARGIN, "{left}");
    }
}
