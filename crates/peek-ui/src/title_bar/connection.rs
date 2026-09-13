//! The connection pill and its picker, ported from
//! `~/labs/peek/src/components/titlebar/ConnectionPicker/` and `src/Connection/WorkspacePopover.tsx`.
//!
//! The pill is tinted by the connection's own colour — 15 % fill behind a 35 % border, both
//! deepening on hover — and the menu groups connections under their workspace, marking the open
//! one. Choosing another loads its document into this window.
//!
//! Two differences from the reference, both forced. There is no `backdrop-filter`, as elsewhere in
//! this crate. And the popover has no keyboard trigger yet: `DropdownMenu` exposes only
//! `on_open_change`, not a controlled `open`, so the reference's bare `p` binding
//! (`ConnectionPicker::Open`, already reserved in `settings.schema.json`) needs a hand-owned
//! `Popover`/`PopupMenu` pair and waits for its own change.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable, Size, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{App, BoxShadow, Entity, Hsla, SharedString, Window, div, point, px, rems};
use peek_config::PeekConfig;
use peek_theme::ActivePeekTheme;

use crate::database::Database;
use crate::workspace::WorkspaceView;

/// `.connection-indicator`, and the dot in front of every row in the menu.
const DOT: gpui_kit::Pixels = px(7.0);
/// The pill, sized to sit inside the 40 px title bar with room to breathe.
const HEIGHT: gpui_kit::Pixels = px(26.0);

/// One selectable connection, snapshotted out of `settings.json` at startup — as the reference's
/// `useGetConfig` does, and so that rendering the pill never touches the disk.
#[derive(Debug, Clone)]
pub(crate) struct Choice {
    workspace: SharedString,
    name: SharedString,
    /// `user@host`, the reference's second line; `None` when the URL has neither.
    origin: Option<SharedString>,
    tint: Option<(u8, u8, u8)>,
}

#[derive(IntoElement, Clone)]
pub(crate) struct ConnectionPicker {
    workspace: Entity<WorkspaceView>,
    open: (SharedString, SharedString),
    choices: Rc<[Choice]>,
}

impl ConnectionPicker {
    pub(crate) fn new(
        workspace: Entity<WorkspaceView>,
        open: (SharedString, SharedString),
        choices: Rc<[Choice]>,
    ) -> Self {
        Self {
            workspace,
            open,
            choices,
        }
    }
}

/// What the pill has to say. A failed connection is not a change of colour alone: the indicator
/// dot becomes a warning glyph, because a status told only in red is no status to a reader who
/// cannot see red.
enum Status {
    /// `settings.json` describes no connections at all.
    Empty,
    Open(Hsla),
    Failed(SharedString),
}

impl RenderOnce for ConnectionPicker {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let error = Database::error(cx).map(SharedString::from);
        let status = status(&self, error.as_ref(), cx);

        let menu_source = self.clone();
        pill(&self.open, status, cx)
            .dropdown_menu(move |menu, _, _| build_menu(menu, &menu_source, error.as_ref()))
    }
}

/// The pill's state. A failure outranks the tint: which connection is open matters less than the
/// fact that it did not open.
fn status(picker: &ConnectionPicker, error: Option<&SharedString>, cx: &App) -> Status {
    if let Some(error) = error {
        return Status::Failed(error.clone());
    }
    if picker.choices.is_empty() {
        return Status::Empty;
    }
    let tint = picker
        .choices
        .iter()
        .find(|choice| choice.workspace == picker.open.0 && choice.name == picker.open.1)
        .and_then(|choice| resolve(choice.tint));
    Status::Open(tint.unwrap_or_else(|| cx.peek_theme().accent))
}

/// The trigger. With nothing configured it still renders — disabled and saying so — rather than
/// vanishing: an empty `settings.json` is a state, not an absence.
///
/// The pill's surface lives on an inner element, not on the `Button`: `Button::render` sets its
/// own hover style, and gpui asserts that only one is set per element.
fn pill(open: &(SharedString, SharedString), status: Status, cx: &App) -> Button {
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
                .tooltip("Add a connection in ~/peek/settings.json");
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

    button
        .child(
            surface(accent, theme.fg, cx)
                .bg(accent.opacity(0.15))
                .border_color(accent.opacity(0.35))
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

/// The reference glows the dot for the active connection and leaves the menu's rows flat.
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

fn build_menu(
    mut menu: PopupMenu,
    picker: &ConnectionPicker,
    error: Option<&SharedString>,
) -> PopupMenu {
    // The failure sits above the rows, next to the connections that are the way out of it: the
    // reference puts it in this same popover.
    if let Some(error) = error {
        menu = menu.item(failure(error)).separator();
    }
    if picker.choices.is_empty() {
        return menu
            .item(PopupMenuItem::new("No connections in ~/peek/settings.json").disabled(true));
    }

    let (open, view) = (&picker.open, &picker.workspace);
    let mut heading: Option<&SharedString> = None;
    for choice in &*picker.choices {
        if heading != Some(&choice.workspace) {
            if heading.is_some() {
                menu = menu.separator();
            }
            menu = menu.item(PopupMenuItem::label(choice.workspace.clone()));
            heading = Some(&choice.workspace);
        }
        menu = menu.item(row(choice, open, view));
    }
    menu
}

/// Why the connection did not open. Driver and tunnel messages already name the recovery
/// ("add it with `ssh-keyscan`…"), so they are shown whole and left to wrap rather than truncated.
fn failure(error: &SharedString) -> PopupMenuItem {
    let error = error.clone();
    PopupMenuItem::element(move |_, cx| {
        let theme = cx.peek_theme();
        div()
            .h_flex()
            .gap(px(10.0))
            .items_start()
            .max_w(rems(22.0))
            .child(
                div().pt(px(2.0)).child(
                    Icon::new(IconName::TriangleAlert)
                        .size(px(11.0))
                        .text_color(theme.red),
                ),
            )
            .child(
                div()
                    .v_flex()
                    .min_w_0()
                    .child(div().child("Couldn't connect"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.fg_subtle)
                            .child(error.clone()),
                    ),
            )
    })
    .disabled(true)
}

fn row(
    choice: &Choice,
    open: &(SharedString, SharedString),
    view: &Entity<WorkspaceView>,
) -> PopupMenuItem {
    let current = choice.workspace == open.0 && choice.name == open.1;
    let (name, origin, tint) = (choice.name.clone(), choice.origin.clone(), choice.tint);
    let (workspace, connection) = (choice.workspace.clone(), choice.name.clone());
    let view = view.clone();

    PopupMenuItem::element(move |_, cx| {
        let theme = cx.peek_theme();
        let color = resolve(tint).unwrap_or(theme.fg_subtle);
        div()
            .h_flex()
            .gap(px(10.0))
            .items_start()
            .min_w(rems(14.0))
            .child(div().pt(px(5.0)).child(dot(color, false)))
            .child(
                div()
                    .v_flex()
                    .min_w_0()
                    .child(div().truncate().child(name.clone()))
                    .children(origin.clone().map(|origin| {
                        div()
                            .text_xs()
                            .text_color(theme.fg_subtle)
                            .truncate()
                            .child(origin)
                    })),
            )
    })
    .checked(current)
    .on_click(move |_, window, cx| {
        view.update(cx, |workspace_view, cx| {
            workspace_view.switch_connection(
                workspace.to_string(),
                connection.to_string(),
                window,
                cx,
            );
        });
    })
}

/// Every connection in the config, flattened in file order so the menu's grouping follows it.
pub(crate) fn choices(config: &PeekConfig) -> Rc<[Choice]> {
    config
        .workspaces
        .iter()
        .flat_map(|workspace| {
            workspace.connections.iter().map(|connection| Choice {
                workspace: SharedString::from(workspace.name.clone()),
                name: SharedString::from(connection.name.clone()),
                origin: connection.origin().map(SharedString::from),
                tint: connection.rgb(),
            })
        })
        .collect()
}

/// The connection's tint as a theme-independent colour; `None` when `settings.json` holds
/// something neither the colour picker nor the seeded defaults wrote.
fn resolve(tint: Option<(u8, u8, u8)>) -> Option<Hsla> {
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
