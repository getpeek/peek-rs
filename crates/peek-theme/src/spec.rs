//! The authoring form of a theme: plain `const`-constructible data, one table per built-in.
//! Colours are `0xRRGGBBAA`; the resolved [`crate::PeekTheme`] converts to gpui at runtime.

use peek_config::ThemeId;

/// An sRGB colour with alpha, packed as `0xRRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color(u32);

impl Color {
    /// An opaque colour from `0xRRGGBB`.
    #[must_use]
    pub const fn rgb(rgb: u32) -> Self {
        Self((rgb << 8) | 0xff)
    }

    /// A colour from `0xRRGGBBAA`.
    #[must_use]
    pub const fn rgba(rgba: u32) -> Self {
        Self(rgba)
    }

    #[must_use]
    pub const fn packed(self) -> u32 {
        self.0
    }

    #[must_use]
    pub fn channels(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    #[must_use]
    pub fn alpha(self) -> f32 {
        f32::from(self.channels()[3]) / 255.0
    }

    /// `#rrggbb` when opaque, `#rrggbbaa` otherwise — what gpui-component parses.
    #[must_use]
    pub fn hex(self) -> String {
        let [r, g, b, a] = self.channels();
        if a == 0xff {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    /// Same colour with a different alpha in `0..=1`.
    #[must_use]
    pub fn with_alpha(self, alpha: f32) -> Self {
        let [r, g, b, _] = self.channels();
        Self(u32::from_be_bytes([r, g, b, unit_to_byte(alpha)]))
    }

    /// Linear sRGB mix towards `other` by `amount` in `0..=1` (`color-mix(in srgb, …)`).
    #[must_use]
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let a = self.channels().map(f32::from);
        let b = other.channels().map(f32::from);
        let mut out = [0u8; 4];
        for index in 0..4 {
            out[index] = unit_to_byte((a[index] + (b[index] - a[index]) * amount) / 255.0);
        }
        Self(u32::from_be_bytes(out))
    }

    /// WCAG relative luminance of the opaque colour.
    #[must_use]
    pub fn luminance(self) -> f64 {
        let [r, g, b, _] = self.channels();
        let linear = |channel: u8| {
            let c = f64::from(channel) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    /// WCAG contrast ratio between two opaque colours.
    #[must_use]
    pub fn contrast(self, other: Self) -> f64 {
        let (light, dark) = {
            let (a, b) = (self.luminance(), other.luminance());
            (a.max(b), a.min(b))
        };
        (light + 0.05) / (dark + 0.05)
    }
}

fn unit_to_byte(value: f32) -> u8 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=255 first"
    )]
    let byte = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    byte
}

pub const WHITE: Color = Color::rgb(0x00ff_ffff);
pub const BLACK: Color = Color::rgb(0x0000_0000);

/// Per-node-kind accents (`nodeTypeColor.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeTypeColors {
    pub query: Color,
    pub agent: Color,
    pub result: Color,
    pub chart: Color,
    pub error: Color,
    pub variable: Color,
    pub activity: Color,
}

/// The node card's frame decoration beyond its border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeFrame {
    Plain,
    /// Eight short segments at the corners (Terminal, Blueprint), in world pixels at zoom 1.
    Brackets {
        color: Color,
        length: f32,
        thickness: f32,
        selected_length: f32,
        selected_thickness: f32,
    },
}

/// How the kind indicator in the node header is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeIndicator {
    Dot,
    Tick,
}

/// The resting node shadow; `None` for flat themes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSpec {
    pub offset_y: f32,
    pub blur: f32,
    pub color: Color,
}

/// The canvas backdrop: a flat base and an optional top-to-bottom gradient over it. The
/// radial glows the CSS themes layered on top have no gpui equivalent yet and are dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasBackground {
    pub base: Color,
    pub gradient: Option<(Color, Color)>,
}

/// Syntax colours for the SQL editor, copied from the Monaco themes. Consumed by the editor
/// milestone; carried here so a theme is complete in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxSpec {
    pub keyword: Color,
    pub keyword_control: Color,
    pub string: Color,
    pub number: Color,
    pub function: Color,
    pub type_name: Color,
    pub variable: Color,
    pub comment: Color,
    pub operator: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeSpec {
    pub id: ThemeId,
    pub name: &'static str,
    pub tagline: &'static str,
    pub is_light: bool,

    pub bg: Color,
    pub bg_grid: Color,
    pub canvas: CanvasBackground,
    pub node_bg: Color,
    pub node_bg_2: Color,
    pub node_inset: Color,
    pub node_border: Color,
    pub node_border_strong: Color,
    pub node_shadow: Option<ShadowSpec>,

    pub fg: Color,
    pub fg_muted: Color,
    pub fg_subtle: Color,

    pub accent: Color,
    pub accent_soft: Color,
    pub accent_bg: Color,
    pub accent_line: Color,
    /// Terminal's phosphor green: the selection colour when it differs from the accent.
    pub active: Option<Color>,
    /// `color-mix` amounts for selected rows: towards white on dark themes, black on light.
    pub row_selected_mix: [f32; 2],

    pub green: Color,
    pub green_soft: Color,
    pub yellow: Color,
    pub yellow_soft: Color,
    pub blue: Color,
    pub blue_soft: Color,
    pub red: Color,
    pub red_soft: Color,
    /// Two further categorical hues the semantic four do not cover, for column types and
    /// badges. Per theme rather than one shared pair: the TypeScript app hardcodes the same
    /// `#e57bb1` and `#4dc3c0` in all six stylesheets, which sits at 2.7:1 and 2.1:1 on the
    /// light themes' node background — below even the muted-text floor.
    pub magenta: Color,
    pub cyan: Color,

    pub regions: [Color; 5],
    pub chart_series: [Color; 5],
    pub node_types: NodeTypeColors,

    pub radius_node: f32,
    pub radius_card: f32,
    pub radius_pill: f32,
    pub node_frame: NodeFrame,
    pub type_indicator: TypeIndicator,

    pub syntax: SyntaxSpec,
}

impl ThemeSpec {
    /// The colour a selected node is outlined with.
    #[must_use]
    pub fn selection(&self) -> Color {
        self.active.unwrap_or(self.accent)
    }

    /// The floating canvas panels' surface: `color-mix(in oklab, var(--pk-node-bg), transparent)`
    /// in `Toolbar.css`, which with no percentages is an even mix — half-opaque `node_bg`.
    #[must_use]
    pub fn chrome(&self) -> Color {
        self.node_bg.with_alpha(0.5)
    }

    /// Selected table-row backgrounds derived the way the CSS `color-mix` did.
    #[must_use]
    pub fn row_selected(&self) -> (Color, Color) {
        let towards = if self.is_light { BLACK } else { WHITE };
        (
            self.node_bg.mix(towards, self.row_selected_mix[0]),
            self.node_bg.mix(towards, self.row_selected_mix[1]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_alpha() {
        assert_eq!(Color::rgb(0x009b_6dff).hex(), "#9b6dff");
        assert_eq!(Color::rgba(0x9b6d_ff24).hex(), "#9b6dff24");
        assert_eq!(Color::rgb(0x009b_6dff).with_alpha(0.5).hex(), "#9b6dff80");
    }

    #[test]
    fn mixing_moves_towards_the_target() {
        let mixed = BLACK.mix(WHITE, 0.5);
        assert_eq!(mixed.channels()[..3], [128, 128, 128]);
        assert!((WHITE.contrast(BLACK) - 21.0).abs() < 0.01);
    }
}
