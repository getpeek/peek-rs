//! The bottom-centre tool palette (`~/labs/peek/src/canvas/ui/Toolbar.tsx`): icon buttons that
//! arm a place tool, each with a badge showing its key.
//!
//! Which tools are live is derived from the command registry rather than listed here. A tool
//! whose action has a [`crate::commands::Command`] entry is enabled, dispatches it, and shows
//! the key that entry is actually bound to; one without an entry renders disabled with a
//! tooltip saying what it is waiting for. That means a tool lights up the moment its command is
//! registered, with no change to this file — and no dead keybinding or palette entry in the
//! meantime.
//!
//! Two deliberate improvements on the reference. Its badges are hardcoded letters in a
//! `ToolDef` array, so rebinding `q` never updates the badge; ours resolve the live binding.
//! And it hardcodes which tools exist, so a tool that is not wired up looks identical to one
//! that is.

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{Disableable, Selectable, StyledExt};
use gpui_kit::component::{Icon, Sizable, Size};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, App, BoxShadow, Context, Div, FocusHandle, Pixels, SharedString, Window, div, point, px,
};
use peek_document::NodeType;
use peek_theme::ActivePeekTheme;

use super::CanvasView;
use crate::commands::{self, actions};

/// `.toolbar-btn`: a 16 px glyph in 7 px / 10 px of padding.
const BUTTON_HEIGHT: Pixels = px(30.0);
const GLYPH: Pixels = px(16.0);

/// `.canvas-toolbar`: `gap: 4px; padding: 5px`, a hairline, and the theme's shadow at the
/// reference's depth.
///
/// **No backdrop blur.** The reference frosts this with `backdrop-filter: blur(14px)` over a
/// half-opaque surface; gpui's only blur is `BoxShadow::blur_radius`, so the canvas shows through
/// unblurred. `title_bar/mod.rs` records the same limitation for the title bar.
///
/// **Why pixels.** These are the reference's constants, and its chrome does not scale with a root
/// font size: at Peek's 13 px rem the nearest component size step renders a 26 px button against
/// the reference's 30 px. Porting the geometry verbatim is the point.
fn panel(cx: &App) -> Div {
    let theme = cx.peek_theme();
    div()
        .h_flex()
        .items_center()
        .gap(px(4.0))
        .p(px(5.0))
        .rounded(theme.radius_card)
        .bg(theme.chrome_bg)
        .border_1()
        .border_color(theme.node_border)
        .when_some(theme.chrome_shadow, |this, (offset_y, blur, color)| {
            this.shadow(vec![BoxShadow {
                color,
                offset: point(px(0.0), offset_y),
                blur_radius: blur,
                spread_radius: px(0.0),
                inset: false,
            }])
        })
}

/// The glyph is a child rather than `Button::icon`, which would resize it to three quarters of the
/// frame (`button.rs`, `icon_size`) and render a 22 px glyph in a 30 px button.
fn icon_button(id: impl Into<SharedString>, icon: IconName) -> Button {
    Button::new(id.into())
        .ghost()
        .with_size(Size::Size(BUTTON_HEIGHT))
        .h(BUTTON_HEIGHT)
        .px(px(10.0))
        .child(Icon::new(icon).size(GLYPH))
}

/// `.sep`: a 1 × 18 px rule between groups of tools.
fn separator(cx: &App) -> Div {
    div()
        .w(px(1.0))
        .h(px(18.0))
        .mx(px(2.0))
        .flex_shrink_0()
        .bg(cx.peek_theme().node_border)
}

/// A slot in the palette. `waiting_for` is `None` for tools that should already work; a tool
/// that is both unregistered and has no reason recorded is a bug, not a placeholder.
struct Tool {
    id: &'static str,
    label: &'static str,
    icon: IconName,
    /// Which kind arms it, so the button can show itself as active. `None` for Select, which is
    /// "no tool armed", and for actions that are not place tools.
    arms: Option<NodeType>,
    waiting_for: Option<&'static str>,
    build: fn() -> Box<dyn Action>,
    /// `.sep` follows this tool: select is its own group, then the place tools.
    separator_after: bool,
}

const TOOLS: &[Tool] = &[
    Tool {
        id: "Tool::Select",
        label: "Select",
        icon: IconName::MousePointer2,
        arms: None,
        waiting_for: None,
        build: || Box::new(actions::tool::Select),
        separator_after: true,
    },
    Tool {
        id: "Tool::Query",
        label: "Query",
        icon: IconName::Code,
        arms: Some(NodeType::Query),
        // Placing and editing a query needs only peek-lsp; running one needs peek-db.
        waiting_for: Some("the query editor"),
        build: || Box::new(actions::tool::Query),
        separator_after: false,
    },
    Tool {
        id: "Tool::Agent",
        label: "Agent",
        icon: IconName::Sparkles,
        arms: Some(NodeType::Agent),
        waiting_for: Some("the agent backend"),
        build: || Box::new(actions::tool::Agent),
        separator_after: false,
    },
    Tool {
        id: "Tool::Text",
        label: "Text",
        icon: IconName::Type,
        arms: Some(NodeType::Text),
        waiting_for: None,
        build: || Box::new(actions::tool::Text),
        separator_after: false,
    },
    Tool {
        id: "Tool::Variable",
        label: "Variable",
        icon: IconName::AtSign,
        arms: Some(NodeType::Variable),
        waiting_for: None,
        build: || Box::new(actions::tool::Variable),
        separator_after: false,
    },
    Tool {
        id: "Tool::Draw",
        label: "Draw",
        icon: IconName::Pencil,
        arms: Some(NodeType::Draw),
        waiting_for: None,
        build: || Box::new(actions::tool::Draw),
        separator_after: false,
    },
];

pub(super) fn render(
    view: &CanvasView,
    window: &mut Window,
    cx: &mut Context<CanvasView>,
) -> impl IntoElement {
    let armed = view.armed_tool();
    let focus = view.focus_handle.clone();

    // A loop rather than `map`: the closure would have to capture `window` mutably and hand
    // back elements that borrow it.
    let mut cells = Vec::with_capacity(TOOLS.len());
    for tool in TOOLS {
        cells.push(button(tool, armed, &focus, window, cx).into_any_element());
        if tool.separator_after {
            cells.push(separator(cx).into_any_element());
        }
    }

    div()
        .id("toolbar")
        .test_support()
        .absolute()
        .bottom_4()
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(panel(cx).children(cells))
}

fn button(
    tool: &'static Tool,
    armed: Option<NodeType>,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &App,
) -> impl IntoElement {
    let command = commands::find(tool.id);
    let active = match tool.arms {
        // Select is the resting state: it is lit when nothing is armed.
        None => armed.is_none(),
        Some(kind) => armed == Some(kind),
    };
    let action = (tool.build)();
    let badge = command.and_then(|_| Kbd::binding_for_action_in(&*action, focus, window));

    let mut button = icon_button(tool.id, tool.icon).selected(active);

    button = match (command, tool.waiting_for) {
        (Some(_), _) => {
            let focus = focus.clone();
            button
                .tooltip_with_action(tool.label, &*action, Some(commands::CANVAS))
                .on_click(move |_, window, cx| {
                    focus.dispatch_action(&*(tool.build)(), window, cx);
                })
        }
        (None, Some(waiting)) => button.disabled(true).tooltip(SharedString::from(format!(
            "{} — waiting on {waiting}",
            tool.label
        ))),
        // Unregistered with no reason recorded: show it plainly rather than inventing one.
        (None, None) => button.disabled(true).tooltip(tool.label),
    };

    div()
        .h_flex()
        .items_center()
        .gap(px(7.0))
        .child(button)
        .children(badge.map(|kbd| key_badge(kbd, cx)))
}

/// `.kbd`: the reference's outlined chip, which `Kbd` already draws — it only wants the
/// transparent fill and the subtler text the canvas chrome uses.
fn key_badge(kbd: Kbd, cx: &App) -> impl IntoElement {
    kbd.outline()
        .bg(gpui_kit::transparent_black())
        .border_color(cx.peek_theme().node_border)
        .text_color(cx.peek_theme().fg_subtle)
}
