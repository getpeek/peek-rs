//! The canvas-level context menu: a list of labelled actions drawn at the pointer.
//!
//! **Chrome, not content.** It is an absolutely positioned sibling of `CanvasElement` rather
//! than something a node raises, which is what `canvas/jump.rs`, `canvas/page_search` and the
//! connection picker all do. That matters here for a specific reason: a `deferred` draw raised
//! from *inside* a node body inherits the node's `BASE_REM * zoom` rem scope — `defer_draw`
//! captures `rem_size` and both deferred passes re-enter `with_rem_size` — so it would grow and
//! shrink with the camera. A menu that doubles in size when you zoom in is content behaving
//! like chrome; the tool palette and the zoom cluster are pinned in pixels for the same reason.
//!
//! Rows carry ids a test can name, which a `PopupMenu`'s index-keyed items do not
//! (`tests/connection.rs`, on why the connection picker is hand-owned too).

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, AnyElement, App, BoxShadow, Context, MouseButton, MouseDownEvent, SharedString, div,
    point, px,
};
use peek_canvas::Point;
use peek_theme::ActivePeekTheme;

use super::CanvasView;

/// `.column-menu-dropdown` and `.column-menu-item`, in pixels because the overlay does not scale.
const ITEM_HEIGHT: f32 = 26.0;
const ITEM_PADDING_X: f32 = 10.0;
const DROPDOWN_PADDING: f32 = 4.0;
const ITEM_RADIUS: f32 = 6.0;
const GLYPH: f32 = 14.0;
/// Kept clear of the pane's edges so a menu opened in the corner is still whole.
const EDGE_MARGIN: f32 = 8.0;
/// Names the row so hovering it can tint the glyph, which `.column-menu-item:hover` does.
const ITEM_GROUP: &str = "context-menu-item";

/// The dropdown's own width, which differs between the two menus in the reference.
pub(crate) const CELL_MENU_WIDTH: f32 = 280.0;
pub(crate) const HEADER_MENU_WIDTH: f32 = 220.0;
/// Three format names need far less room than the menu that opens them.
const SUBMENU_WIDTH: f32 = 120.0;

/// One row. Every leaf is a registered command, so the menu needs no closures and holds no
/// borrow of the node that raised it.
pub(crate) struct MenuItem {
    pub(crate) id: SharedString,
    pub(crate) label: SharedString,
    pub(crate) icon: Option<IconName>,
    /// `None` for a row that only opens a submenu.
    pub(crate) action: Option<Box<dyn Action>>,
    /// Opens beside the row on hover, as Mantine's `Menu.Sub` does. Non-empty means the row is
    /// a heading rather than a command.
    pub(crate) submenu: Vec<MenuItem>,
    /// Drawn in the danger colour and behind a rule, as the reference's `color='red'` Delete is.
    pub(crate) danger: bool,
}

impl MenuItem {
    pub(crate) fn command(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: IconName,
        action: Box<dyn Action>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: Some(icon),
            action: Some(action),
            submenu: Vec::new(),
            danger: false,
        }
    }

    pub(crate) fn submenu(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: IconName,
        submenu: Vec<Self>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: Some(icon),
            action: None,
            submenu,
            danger: false,
        }
    }

    #[must_use]
    pub(crate) fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

/// A row that is not a row: the column name above the header menu's two items.
pub(crate) enum MenuEntry {
    Label(SharedString),
    Separator,
    Item(Box<MenuItem>),
}

pub(crate) struct MenuState {
    /// Pane coordinates of the press that opened it.
    pub(crate) at: Point,
    pub(crate) width: f32,
    pub(crate) entries: Vec<MenuEntry>,
    /// Which row's submenu is open, by entry index. Hovering any row closes the last one, so at
    /// most one is ever up — the behaviour Mantine's `registerOpenSub` gives for free.
    pub(crate) open_submenu: Option<usize>,
}

impl std::fmt::Debug for MenuState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MenuState")
            .field("at", &self.at)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl MenuState {
    /// How tall the menu will be, so it can be flipped rather than run off the bottom.
    fn height(&self) -> f32 {
        let rows: f32 = self
            .entries
            .iter()
            .map(|entry| match entry {
                // A label is a row like any other, just one that does nothing.
                MenuEntry::Item(_) | MenuEntry::Label(_) => ITEM_HEIGHT,
                MenuEntry::Separator => DROPDOWN_PADDING * 2.0 + 1.0,
            })
            .sum();
        rows + DROPDOWN_PADDING * 2.0
    }

    /// Where the dropdown's top-left goes, given the pane it has to stay inside.
    ///
    /// The reference leans on floating-ui's shift middleware; the rule here is the same one:
    /// prefer down-and-right of the pointer, flip to the other side when there is no room, and
    /// never leave the pane.
    fn origin(&self, pane: (f32, f32)) -> (f32, f32) {
        let (pane_width, pane_height) = pane;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a pointer position in pane pixels is far inside f32"
        )]
        let (x, y) = (self.at.x as f32, self.at.y as f32);
        let height = self.height();

        let left = if x + self.width + EDGE_MARGIN > pane_width {
            (x - self.width).max(EDGE_MARGIN)
        } else {
            x
        };
        let top = if y + height + EDGE_MARGIN > pane_height {
            (y - height).max(EDGE_MARGIN)
        } else {
            y
        };
        (left, top)
    }
}

pub(super) fn render(view: &CanvasView, cx: &mut Context<CanvasView>) -> Option<AnyElement> {
    let menu = view.context_menu.as_ref()?;
    let pane = view.pane_size_for_menu();
    let (left, top) = menu.origin(pane);
    let theme = cx.peek_theme().clone();

    let rows: Vec<AnyElement> = menu
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| match entry {
            MenuEntry::Label(text) => label(text, cx),
            MenuEntry::Separator => separator(cx),
            MenuEntry::Item(item) => row(item, index, menu, cx),
        })
        .collect();

    Some(
        div()
            .absolute()
            .inset_0()
            // A press anywhere else closes it, which is the whole of the dismissal rule — the
            // same scrim `jump.rs` uses, minus the dim.
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    view.close_context_menu(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    view.close_context_menu(cx);
                }),
            )
            .child(
                div()
                    .id("context-menu")
                    .test_support()
                    // The scrim dismisses on mouse *down*, so without the dropdown claiming its
                    // own presses the menu would be gone before the row's click completed — and
                    // every item would silently do nothing.
                    .on_mouse_down(
                        MouseButton::Left,
                        |_: &MouseDownEvent, _: &mut gpui_kit::Window, cx: &mut App| {
                            cx.stop_propagation();
                        },
                    )
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(menu.width))
                    .p(px(DROPDOWN_PADDING))
                    .rounded(theme.radius_card)
                    .border_1()
                    .border_color(theme.node_border)
                    .bg(theme.node_bg)
                    .when_some(theme.node_shadow, |this, (offset_y, blur, color)| {
                        this.shadow(vec![BoxShadow {
                            color,
                            offset: point(px(0.0), offset_y),
                            blur_radius: blur,
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                    })
                    .children(rows),
            )
            .into_any_element(),
    )
}

fn row(
    item: &MenuItem,
    index: usize,
    menu: &MenuState,
    cx: &mut Context<CanvasView>,
) -> AnyElement {
    let theme = cx.peek_theme().clone();
    let ink = if item.danger { theme.red } else { theme.fg };
    let glyph = if item.danger {
        theme.red
    } else {
        theme.fg_muted
    };
    let has_submenu = !item.submenu.is_empty();
    let open = has_submenu && menu.open_submenu == Some(index);
    let action = item.action.as_ref().map(|action| action.boxed_clone());

    div()
        .id(gpui_kit::ElementId::from(item.id.clone()))
        .test_support()
        .aria_label(item.label.clone())
        .relative()
        .h_flex()
        .items_center()
        .gap(px(8.0))
        .h(px(ITEM_HEIGHT))
        .px(px(ITEM_PADDING_X))
        .rounded(px(ITEM_RADIUS))
        .text_size(px(12.0))
        .text_color(ink)
        .cursor_pointer()
        .group(ITEM_GROUP)
        .when(open, |row| row.bg(theme.node_bg_2))
        .hover(|row| row.bg(theme.node_bg_2))
        // Hovering any row closes whichever submenu was up, so only one is ever open — and a
        // heading opens its own on the way past, without needing a click.
        .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
            if *hovered {
                view.set_open_submenu(has_submenu.then_some(index), cx);
            }
        }))
        .children(item.icon.map(|icon| {
            div()
                .flex_none()
                .text_color(glyph)
                .group_hover(ITEM_GROUP, move |glyph| glyph.text_color(theme.accent_soft))
                .child(Icon::new(icon).size(px(GLYPH)))
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(item.label.clone()),
        )
        .when(has_submenu, |row| {
            row.child(
                div()
                    .flex_none()
                    .text_color(glyph)
                    .child(Icon::new(IconName::ChevronRight).size(px(GLYPH))),
            )
        })
        .when_some(action, |row, action| {
            row.on_click(cx.listener(move |view, _, window, cx| {
                view.close_context_menu(cx);
                window.dispatch_action(action.boxed_clone(), cx);
            }))
        })
        .when(open, |row| row.child(submenu(item, menu.width, cx)))
        .into_any_element()
}

/// The child list, drawn beside its heading rather than below it.
///
/// It is a child of the row, so it inherits the dropdown's own stacking and needs no second
/// overlay — `position: right-start` with `offset: 0`, as Mantine's `MenuSub` defaults have it.
fn submenu(item: &MenuItem, parent_width: f32, cx: &mut Context<CanvasView>) -> AnyElement {
    let theme = cx.peek_theme().clone();
    let rows: Vec<AnyElement> = item.submenu.iter().map(|child| leaf(child, cx)).collect();

    div()
        // Positioned beyond the dropdown's own right edge, so it is outside that element's
        // hitbox and has to claim presses for itself or the scrim dismisses the menu before the
        // click lands.
        .on_mouse_down(
            MouseButton::Left,
            |_: &MouseDownEvent, _: &mut gpui_kit::Window, cx: &mut App| {
                cx.stop_propagation();
            },
        )
        .absolute()
        .left(px(parent_width - ITEM_PADDING_X * 2.0 - DROPDOWN_PADDING))
        .top(px(-DROPDOWN_PADDING))
        .w(px(SUBMENU_WIDTH))
        .p(px(DROPDOWN_PADDING))
        .rounded(theme.radius_card)
        .border_1()
        .border_color(theme.node_border)
        .bg(theme.node_bg)
        .when_some(theme.node_shadow, |this, (offset_y, blur, color)| {
            this.shadow(vec![BoxShadow {
                color,
                offset: point(px(0.0), offset_y),
                blur_radius: blur,
                spread_radius: px(0.0),
                inset: false,
            }])
        })
        .children(rows)
        .into_any_element()
}

/// A submenu row, which is always a command.
fn leaf(item: &MenuItem, cx: &mut Context<CanvasView>) -> AnyElement {
    let theme = cx.peek_theme().clone();
    let action = item.action.as_ref().map(|action| action.boxed_clone());

    div()
        .id(gpui_kit::ElementId::from(item.id.clone()))
        .test_support()
        .aria_label(item.label.clone())
        .h_flex()
        .items_center()
        .gap(px(8.0))
        .h(px(ITEM_HEIGHT))
        .px(px(ITEM_PADDING_X))
        .rounded(px(ITEM_RADIUS))
        .text_size(px(12.0))
        .text_color(theme.fg)
        .cursor_pointer()
        .group(ITEM_GROUP)
        .hover(|row| row.bg(theme.node_bg_2))
        .children(item.icon.map(|icon| {
            div()
                .flex_none()
                .text_color(theme.fg_muted)
                .group_hover(ITEM_GROUP, move |glyph| glyph.text_color(theme.accent_soft))
                .child(Icon::new(icon).size(px(GLYPH)))
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(item.label.clone()),
        )
        .when_some(action, |row, action| {
            row.on_click(cx.listener(move |view, _, window, cx| {
                log::error!("PROBE leaf click");
                view.close_context_menu(cx);
                window.dispatch_action(action.boxed_clone(), cx);
            }))
        })
        .into_any_element()
}

fn label(text: &SharedString, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    div()
        .h_flex()
        .items_center()
        .h(px(ITEM_HEIGHT))
        .px(px(ITEM_PADDING_X))
        .text_size(px(11.0))
        .text_color(theme.fg_subtle)
        .truncate()
        // `.column-menu-label { text-transform: lowercase }`: the column name is a caption about
        // the menu, not a value in it.
        .child(text.to_lowercase())
        .into_any_element()
}

fn separator(cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    div()
        .h(px(1.0))
        .my(px(DROPDOWN_PADDING))
        .bg(theme.node_border)
        .into_any_element()
}
