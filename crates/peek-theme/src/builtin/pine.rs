#![allow(
    clippy::unreadable_literal,
    reason = "colour literals are hex triplets, which separators would obscure"
)]

use peek_config::ThemeId;

use crate::spec::{
    CanvasBackground, Color, NodeFrame, NodeTypeColors, ShadowSpec, SyntaxSpec, ThemeSpec,
    TypeIndicator,
};

// Converted from the oklch values in `pine.css` (CSS Color 4 gamut clipping); the original
// literal follows each field.
/// The default theme: purple-tinted dark.
pub(super) static PINE: ThemeSpec = ThemeSpec {
    id: ThemeId::Pine,
    name: "Pine",
    tagline: "Purple-tinted dark",
    is_light: false,

    bg: Color::rgb(0x04040f),      // oklch(11.5% 0.028 280)
    bg_grid: Color::rgb(0x1f2138), // oklch(25.8% 0.043 279.1)
    canvas: CanvasBackground {
        base: Color::rgb(0x02020a), // oklch(9% 0.03 281)
        gradient: Some((Color::rgb(0x090917), Color::rgb(0x03030e))),
    },
    node_bg: Color::rgb(0x131428),            // oklch(20.2% 0.04 280)
    node_bg_2: Color::rgb(0x1c1e36),          // oklch(24.6% 0.046 278.9)
    node_inset: Color::rgb(0x0b0b1b),         // oklch(16% 0.034 280)
    node_border: Color::rgb(0x2a2d4a),        // oklch(30.9% 0.052 278.4)
    node_border_strong: Color::rgb(0x3a3d5e), // oklch(37.2% 0.057 279.3)
    node_shadow: Some(ShadowSpec {
        offset_y: 8.0,
        blur: 24.0,
        color: Color::rgba(0x00000080),
    }),

    fg: Color::rgb(0xe6e4f5),        // oklch(92.5% 0.023 291.4)
    fg_muted: Color::rgb(0x9892b0),  // oklch(67.5% 0.044 294.4)
    fg_subtle: Color::rgb(0x5a5878), // oklch(47.5% 0.051 287.1)

    accent: Color::rgb(0x9b6dff),         // oklch(65.2% 0.208 294.4)
    accent_soft: Color::rgb(0xdccafd),    // oklch(87% 0.072 301)
    accent_bg: Color::rgba(0x9b6dff24),   // accent / 0.14
    accent_line: Color::rgba(0x9b6dff80), // accent / 0.5
    active: None,
    row_selected_mix: [0.09, 0.14],

    green: Color::rgb(0x4da66e), // oklch(65.5% 0.12 153.8)
    green_soft: Color::rgba(0x4da66e2e),
    yellow: Color::rgb(0xf0c25c), // oklch(83.5% 0.13 84.9)
    yellow_soft: Color::rgba(0xf0c25c29),
    blue: Color::rgb(0x6b8afd), // oklch(66.5% 0.173 269.8)
    blue_soft: Color::rgba(0x6b8afd29),
    red: Color::rgb(0xe0526a), // oklch(63.4% 0.177 13.6)
    red_soft: Color::rgba(0xe0526a29),
    magenta: Color::rgb(0xe085bd), // oklch(73.6% 0.137 342.8)
    cyan: Color::rgb(0x4fc7c3),    // oklch(74.3% 0.107 189.7)

    regions: [
        Color::rgb(0x9f6fe2), // oklch(64% 0.17 301)
        Color::rgb(0x6b8afd),
        Color::rgb(0xf0c25c),
        Color::rgb(0x4da66e),
        Color::rgb(0xd76695), // oklch(66% 0.15 356)
    ],
    chart_series: [
        Color::rgb(0x9b6dff),
        Color::rgb(0x6b8afd),
        Color::rgb(0x4da66e),
        Color::rgb(0xf0c25c),
        Color::rgb(0xe0526a),
    ],
    node_types: NodeTypeColors {
        query: Color::rgb(0xb07be6),    // oklch(68% 0.16 305)
        agent: Color::rgb(0x497ce6),    // oklch(60.5% 0.17 263)
        result: Color::rgb(0x68c58e),   // oklch(75% 0.12 156)
        chart: Color::rgb(0xe2df50),    // oklch(88% 0.16 108)
        error: Color::rgb(0xe0526a),    // oklch(63.4% 0.177 13.6)
        variable: Color::rgb(0xa77d8b), // oklch(63.5% 0.055 357)
        activity: Color::rgb(0x3fc0c0), // oklch(74% 0.11 195)
    },

    radius_node: 10.0,
    radius_card: 10.0,
    radius_pill: 999.0,
    node_frame: NodeFrame::Plain,
    type_indicator: TypeIndicator::Dot,

    // Rosé Pine (Monaco `rose-pine`).
    syntax: SyntaxSpec {
        keyword: Color::rgb(0x3e8fb0),
        keyword_control: Color::rgb(0x31748f),
        string: Color::rgb(0xf6c177),
        number: Color::rgb(0xea9a97),
        function: Color::rgb(0xeb6f92),
        type_name: Color::rgb(0xeb6f92),
        variable: Color::rgb(0xebbcba),
        comment: Color::rgb(0x6e6a86),
        operator: Color::rgb(0x908caa),
    },
};
