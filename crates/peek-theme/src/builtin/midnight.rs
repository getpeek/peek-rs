#![allow(
    clippy::unreadable_literal,
    reason = "colour literals are hex triplets, which separators would obscure"
)]

use peek_config::ThemeId;

use crate::spec::{
    CanvasBackground, Color, NodeFrame, NodeTypeColors, ShadowSpec, SyntaxSpec, ThemeSpec,
    TypeIndicator,
};

// Converted from the oklch values in `midnight.css`.
/// Pure dark.
pub(super) static MIDNIGHT: ThemeSpec = ThemeSpec {
    id: ThemeId::Midnight,
    name: "Midnight",
    tagline: "Pure dark",
    is_light: false,

    bg: Color::rgb(0x0e0d12),         // oklch(16.3% 0.011 294.4)
    bg_grid: Color::rgba(0xffffff1f), // CanvasBackground.tsx: white, dialled back to 0.12
    canvas: CanvasBackground {
        base: Color::rgb(0x0e0d12),
        gradient: Some((Color::rgb(0x14121a), Color::rgb(0x0e0d12))),
    },
    node_bg: Color::rgb(0x16141c),     // oklch(19.7% 0.016 296.4)
    node_bg_2: Color::rgb(0x1c1a24),   // oklch(22.5% 0.019 293.9)
    node_inset: Color::rgb(0x0d0b11),  // oklch(15.5% 0.013 296.4)
    node_border: Color::rgb(0x2a2733), // oklch(28.1% 0.022 296.4)
    node_border_strong: Color::rgb(0x3a3645), // oklch(34.3% 0.026 297.2)
    node_shadow: Some(ShadowSpec {
        offset_y: 8.0,
        blur: 24.0,
        color: Color::rgba(0x00000059),
    }),

    fg: Color::rgb(0xe8e5f0),        // oklch(92.7% 0.015 298.6)
    fg_muted: Color::rgb(0x9c97ab),  // oklch(68.7% 0.03 297.1)
    fg_subtle: Color::rgb(0x5e5a6b), // oklch(47.8% 0.027 296.1)

    accent: Color::rgb(0x9354e0),      // oklch(58.7% 0.205 301)
    accent_soft: Color::rgb(0xdbcaf9), // oklch(86.8% 0.067 301.5)
    accent_bg: Color::rgba(0x9354e024),
    accent_line: Color::rgba(0x9354e080),
    active: None,
    row_selected_mix: [0.04, 0.08],

    green: Color::rgb(0x4da660), // oklch(65.3% 0.134 148.6)
    green_soft: Color::rgba(0x4da6602e),
    yellow: Color::rgb(0xf6c945), // oklch(85.3% 0.152 89.5)
    yellow_soft: Color::rgba(0xf6c94529),
    blue: Color::rgb(0x6b8afd), // oklch(66.5% 0.173 269.8)
    blue_soft: Color::rgba(0x6b8afd29),
    red: Color::rgb(0xe24a35), // oklch(62% 0.192 30.9)
    red_soft: Color::rgba(0xe24a3529),
    magenta: Color::rgb(0xe57bb1), // oklch(71.8% 0.147 345.4)
    cyan: Color::rgb(0x4dc3c0),    // oklch(73.1% 0.104 190.5)

    regions: [
        Color::rgb(0x9969db), // oklch(62% 0.17 301)
        Color::rgb(0x6b8afd),
        Color::rgb(0xf6c945),
        Color::rgb(0x4da660),
        Color::rgb(0xcd648f), // oklch(64% 0.14 356)
    ],
    chart_series: [
        Color::rgb(0x9354e0),
        Color::rgb(0x6b8afd),
        Color::rgb(0x4da660),
        Color::rgb(0xf6c945),
        Color::rgb(0xe24a35),
    ],
    node_types: NodeTypeColors {
        query: Color::rgb(0x9354e0),
        agent: Color::rgb(0x6b8afd),
        result: Color::rgb(0x4da660),
        chart: Color::rgb(0xf6c945),
        error: Color::rgb(0xe24a35),
        variable: Color::rgb(0xa6808c), // oklch(64% 0.05 357)
        activity: Color::rgb(0x37b2b4), // oklch(70% 0.105 196)
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
