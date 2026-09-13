//! The About dialog (`~/labs/peek/src/command-palette/details/AboutDetails.tsx`).
//!
//! The reference shows this as a detail strip beside the palette list, with an empty
//! `onSelect`: selecting the row does nothing, the information is the row. Peek-rs's palette
//! has no detail strip, so the command opens a small dialog instead and the row behaves like
//! every other one.

use gpui_kit::component::{ActiveTheme, StyledExt, WindowExt};
use gpui_kit::prelude::*;
use gpui_kit::{App, TestSupportExt, Window, div, px, rems};

/// The reference's one-line description of the product, verbatim.
const TAGLINE: &str = "Local-first database canvas";

pub(crate) fn open(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, _| {
        dialog.w(px(300.0)).content(|content, _, cx| {
            content.child(
                div()
                    .id("about")
                    .test_support()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rems(1.15))
                            .font_semibold()
                            .text_color(cx.theme().foreground)
                            .child("Peek"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(TAGLINE),
                    )
                    .child(
                        div()
                            .pt_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                    ),
            )
        })
    });
}
