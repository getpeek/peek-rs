//! The window's title bar: page tabs on the left, the connection picker and palette button on
//! the right, and the window's drag region across the gap between them.
//!
//! Ported from `~/labs/peek/src/components/titlebar/CustomTitleBar.tsx`. Two deliberate
//! differences from the reference, both forced:
//!
//! - **The traffic lights are macOS' own.** The reference draws its own three circles because
//!   Tauri ran with `decorations: false` and left it no choice. Suppressing the native buttons
//!   here would need `NSWindow.standardWindowButton(_:)?.setHidden(true)` — `unsafe` plus an
//!   `AppKit` dependency inside `peek-ui`, both of which `CLAUDE.md` forbids — and
//!   `window_control_area(Close|Min|Max)` is an empty function on macOS, so we would be
//!   reimplementing platform chrome by hand. We keep the real buttons and inset for them.
//! - **No backdrop blur.** The reference's bar is invisible until hover, when it gains
//!   `backdrop-filter: blur(20px)`. gpui's only blur is `BoxShadow::blur_radius`, so the bar
//!   occludes what is behind it rather than frosting it; hover changes the surface and the
//!   hairline instead.

pub(crate) mod close_page;
pub(crate) mod connection;
pub(crate) mod pages;
pub(crate) mod picker;

use gpui_kit::component::{ActiveTheme, StyledExt, TitleBar};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, FocusHandle, Pixels, TitlebarOptions, Window, WindowOptions, div, point, px, rems,
};

use crate::commands::actions;
use crate::settings::Settings;
use connection::ConnectionPill;
use pages::PageTabs;

/// The reference's bar is 50 px carrying the collaborate controls we do not have yet; 40 px is a
/// 26 px pill with room to breathe.
pub(crate) const HEIGHT: Pixels = px(40.0);

/// macOS reserves the top-left for the traffic lights. gpui-component's own constant is 80 px
/// against its 34 px bar; ours is taller, so the buttons sit lower and need the same clearance.
const TRAFFIC_LIGHT_INSET: Pixels = px(80.0);

/// Window options with the traffic lights centred in a [`HEIGHT`] bar, at the reference's 12 px
/// left inset.
pub(crate) fn window_options() -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            traffic_light_position: Some(point(px(12.0), px(13.0))),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    }
}

#[derive(IntoElement)]
pub(crate) struct PeekTitleBar {
    pill: ConnectionPill,
    pages: Entity<PageTabs>,
    canvas_focus: FocusHandle,
}

impl PeekTitleBar {
    pub(crate) fn new(
        pill: ConnectionPill,
        pages: Entity<PageTabs>,
        canvas_focus: FocusHandle,
    ) -> Self {
        Self {
            pill,
            pages,
            canvas_focus,
        }
    }
}

impl RenderOnce for PeekTitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // In fullscreen macOS takes the traffic lights back, so the inset would be a gap.
        let left = if window.is_fullscreen() {
            px(12.0)
        } else {
            TRAFFIC_LIGHT_INSET
        };
        let palette_button_shown = Settings::get(cx)
            .ui
            .titlebar
            .command_palette_button
            .is_shown();
        let canvas_focus = self.canvas_focus.clone();

        TitleBar::new()
            .h(HEIGHT)
            .pl(left)
            .pr(rems(0.75))
            .bg(cx.theme().title_bar)
            .border_color(cx.theme().title_bar_border)
            .child(self.pages.clone())
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .child(self.pill)
                    // `Settings::ToggleCommandPaletteButton`: the palette keeps working from
                    // its shortcut, so this is a preference about chrome, not about the
                    // command.
                    .when(palette_button_shown, move |this| {
                        this.child(palette_button(&canvas_focus))
                    }),
            )
    }
}

/// The reference's expanding search pill. Ours is a plain ghost button; the shortcut lives in
/// its tooltip, which resolves the live binding rather than a hardcoded string.
fn palette_button(canvas_focus: &FocusHandle) -> impl IntoElement {
    use gpui_kit::component::Sizable;
    use gpui_kit::component::button::{Button, ButtonVariants};

    let focus = canvas_focus.clone();
    Button::new("command-palette")
        .ghost()
        .xsmall()
        .label("Search")
        .tooltip_with_action(
            "Command palette",
            &actions::command_palette::Open,
            Some(crate::commands::WORKSPACE),
        )
        .on_click(move |_, window, cx| {
            focus.dispatch_action(&actions::command_palette::Open, window, cx);
        })
}
