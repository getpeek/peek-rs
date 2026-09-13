//! `DrawData.color` is document data, not a theme role: the frozen format stores the CSS colour
//! the stroke was drawn with, which `useDrawTool.ts` writes as `var(--pk-fg)` and `defaults.ts`
//! as `white`. Rendering a stroke faithfully means parsing that string rather than picking a
//! role for it; anything this does not recognise falls back to the theme's foreground so a
//! stroke is never invisible.

use gpui_kit::{Hsla, black, rgb, rgba, white};
use peek_theme::PeekTheme;

pub(crate) fn resolve(color: &str, theme: &PeekTheme) -> Hsla {
    let color = color.trim();
    token(color, theme)
        .or_else(|| named(color))
        .or_else(|| hex(color))
        .unwrap_or(theme.fg)
}

/// A stroke drawn in a theme colour follows the theme, so the token is resolved every frame
/// rather than baked in when the stroke was made.
fn token(color: &str, theme: &PeekTheme) -> Option<Hsla> {
    let name = color.strip_prefix("var(--pk-")?.strip_suffix(')')?;
    match name {
        "fg" => Some(theme.fg),
        "fg-muted" => Some(theme.fg_muted),
        "fg-subtle" => Some(theme.fg_subtle),
        "accent" => Some(theme.accent),
        "red" => Some(theme.red),
        "green" => Some(theme.green),
        "blue" => Some(theme.blue),
        "yellow" => Some(theme.yellow),
        _ => None,
    }
}

fn named(color: &str) -> Option<Hsla> {
    match color {
        "white" => Some(white()),
        "black" => Some(black()),
        _ => None,
    }
}

fn hex(color: &str) -> Option<Hsla> {
    let digits = color.strip_prefix('#')?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        3 => Some(rgb(expanded(value)).into()),
        6 => Some(rgb(value).into()),
        8 => Some(rgba(value).into()),
        _ => None,
    }
}

/// `#abc` means `#aabbcc`.
fn expanded(value: u32) -> u32 {
    let nibble = |shift: u32| (value >> shift) & 0xf;
    let (red, green, blue) = (nibble(8), nibble(4), nibble(0));
    (red << 20) | (red << 16) | (green << 12) | (green << 8) | (blue << 4) | blue
}
