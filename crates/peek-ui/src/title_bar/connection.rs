//! The connection pill: the title bar's trigger for the picker, ported from
//! `~/labs/peek/src/components/titlebar/ConnectionPicker/ConnectionPicker.css`.
//!
//! The pill is tinted by the connection's own colour — 15 % fill behind a 35 % border, both
//! deepening on hover — and says which workspace and connection the window is looking at. The
//! list it opens lives in [`super::picker`].
//!
//! Two differences from the reference, both forced. There is no `backdrop-filter`, as elsewhere
//! in this crate. And a failed connection is not a change of colour alone: the indicator becomes
//! a warning glyph, because a status told only in red is no status to a reader who cannot see
//! red. The driver's own message rides in the tooltip, which is where the reference puts it.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable, Size, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{App, BoxShadow, FocusHandle, Hsla, SharedString, Window, div, point, px};
use peek_theme::ActivePeekTheme;

use crate::commands::actions;
use crate::database::Database;
use crate::settings::Settings;

/// `.connection-indicator`.
const DOT: gpui_kit::Pixels = px(7.0);
/// The pill, sized to sit inside the 40 px title bar with room to breathe.
const HEIGHT: gpui_kit::Pixels = px(26.0);

#[derive(IntoElement)]
pub(crate) struct ConnectionPill {
    open: (SharedString, SharedString),
    /// Whether the panel it triggers is showing, so the pill can hold a pressed state while its
    /// popup is up rather than snapping back to rest under the pointer.
    showing: bool,
    /// Chrome dispatches through the canvas, so the button takes the same path the keyboard does.
    canvas_focus: FocusHandle,
}

impl ConnectionPill {
    pub(crate) fn new(
        open: (SharedString, SharedString),
        showing: bool,
        canvas_focus: FocusHandle,
    ) -> Self {
        Self {
            open,
            showing,
            canvas_focus,
        }
    }
}

/// What the pill has to say.
enum Status {
    /// `settings.json` describes no connections at all.
    Empty,
    Open(Hsla),
    Failed(SharedString),
}

impl RenderOnce for ConnectionPill {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let status = status(&self.open, cx);
        let focus = self.canvas_focus.clone();
        pill(&self.open, (status, self.showing), cx).on_click(move |_, window, cx| {
            focus.dispatch_action(&actions::connection_picker::Open, window, cx);
        })
    }
}

/// The pill's state. A failure outranks the tint: which connection is open matters less than
/// the fact that it did not open.
fn status(open: &(SharedString, SharedString), cx: &App) -> Status {
    if let Some(error) = Database::error(cx) {
        return Status::Failed(SharedString::from(error));
    }
    let config = Settings::get(cx);
    if config.workspaces.iter().all(|w| w.connections.is_empty()) {
        return Status::Empty;
    }
    let tint = config
        .workspaces
        .iter()
        .find(|workspace| workspace.name == open.0.as_str())
        .and_then(|workspace| {
            workspace
                .connections
                .iter()
                .find(|connection| connection.name == open.1.as_str())
        })
        .and_then(|connection| resolve(connection.rgb()));
    Status::Open(tint.unwrap_or_else(|| cx.peek_theme().accent))
}

/// The trigger. With nothing configured it still renders — saying so — rather than vanishing:
/// an empty `settings.json` is a state, not an absence, and the panel is still the way to add
/// the first connection.
///
/// The pill's surface lives on an inner element, not on the `Button`: `Button::render` sets its
/// own hover style, and gpui asserts that only one is set per element.
fn pill(open: &(SharedString, SharedString), state: (Status, bool), cx: &App) -> Button {
    let (status, showing) = state;
    let theme = cx.peek_theme();
    let button = Button::new("connection-picker")
        .ghost()
        .with_size(Size::Size(HEIGHT))
        .h(HEIGHT)
        .p_0()
        .rounded(theme.radius_pill);

    let (accent, indicator, tooltip) = match status {
        Status::Empty => {
            return button
                .child(
                    surface(theme.node_border, theme.fg_muted, cx)
                        .child(div().text_xs().child("No connection")),
                )
                .tooltip("Add a connection");
        }
        Status::Open(accent) => (
            accent,
            dot(accent, true).into_any_element(),
            SharedString::from("Switch connection"),
        ),
        Status::Failed(message) => (
            theme.red,
            Icon::new(IconName::TriangleAlert)
                .size(px(11.0))
                .text_color(theme.red)
                .into_any_element(),
            message,
        ),
    };

    // Open reads as the deepest of the three rest states, so the pill stays visibly pressed
    // while its panel is up instead of dropping back the moment the pointer leaves.
    let (fill, edge) = if showing { (0.25, 0.55) } else { (0.15, 0.35) };

    button
        .child(
            surface(accent.opacity(edge), theme.fg, cx)
                .bg(accent.opacity(fill))
                .hover(move |style| {
                    style
                        .bg(accent.opacity(0.25))
                        .border_color(accent.opacity(0.55))
                })
                .child(indicator)
                .child(
                    div()
                        .text_xs()
                        .min_w_0()
                        .truncate()
                        .h_flex()
                        .child(div().child(open.0.clone()))
                        .child(div().text_color(theme.fg_subtle).child(" / "))
                        .child(div().child(open.1.clone())),
                )
                .child(Icon::new(IconName::ChevronDown).size(px(8.0))),
        )
        .tooltip(tooltip)
}

/// `.connection-button`: `padding: 6px 12px 6px 10px`, gap 8, a pill border.
fn surface(border: Hsla, foreground: Hsla, cx: &App) -> gpui_kit::Div {
    div()
        .h_flex()
        .gap(px(8.0))
        .h_full()
        .pl(px(10.0))
        .pr(px(12.0))
        .rounded(cx.peek_theme().radius_pill)
        .border_1()
        .border_color(border)
        .text_color(foreground)
}

/// The reference glows the dot for the active connection and leaves the panel's rows flat.
fn dot(color: Hsla, glow: bool) -> impl IntoElement {
    div()
        .size(DOT)
        .flex_shrink_0()
        .rounded_full()
        .bg(color)
        .when(glow, move |this| {
            this.shadow(vec![BoxShadow {
                color,
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(8.0),
                spread_radius: px(0.0),
                inset: false,
            }])
        })
}

/// A connection's tint as a theme-independent colour; `None` when `settings.json` holds
/// something neither the colour picker nor the seeded defaults wrote.
pub(crate) fn resolve(tint: Option<(u8, u8, u8)>) -> Option<Hsla> {
    let (red, green, blue) = tint?;
    Some(
        gpui_kit::Rgba {
            r: f32::from(red) / 255.0,
            g: f32::from(green) / 255.0,
            b: f32::from(blue) / 255.0,
            a: 1.0,
        }
        .into(),
    )
}
