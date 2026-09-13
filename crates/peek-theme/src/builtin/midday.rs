#![allow(
    clippy::unreadable_literal,
    reason = "colour literals are hex triplets, which separators would obscure"
)]

use peek_config::ThemeId;

use crate::spec::{
    CanvasBackground, Color, NodeFrame, NodeTypeColors, ShadowSpec, SyntaxSpec, ThemeSpec,
    TypeIndicator,
};

// Converted from the oklch values in `midday.css`.
/// The light theme.
pub(super) static MIDDAY: ThemeSpec = ThemeSpec {
    id: ThemeId::Midday,
    name: "Midday",
    tagline: "Light",
    is_light: true,

    bg: Color::rgb(0xf7f6fc), // oklch(97.6% 0.008 293.9)
    // The CSS says `oklch(78% 0.8 293.7)`, a far out-of-gamut typo for `0.08` that browsers
    // clip to magenta; the accent, dialled back, is the lavender it meant.
    bg_grid: Color::rgba(0x6b3fc43d), // accent / 0.24
    canvas: CanvasBackground {
        base: Color::rgb(0xf5f4fa), // oklch(97% 0.008 293.9)
        gradient: Some((Color::rgb(0xfaf9fd), Color::rgb(0xecebf3))),
    },
    node_bg: Color::rgb(0xffffff),
    node_bg_2: Color::rgb(0xf0eef7),   // oklch(95.3% 0.012 296.3)
    node_inset: Color::rgb(0xeae8f2),  // oklch(93.6% 0.013 296.3)
    node_border: Color::rgb(0xd6d3e2), // oklch(87.4% 0.021 295.1)
    node_border_strong: Color::rgb(0xb1adc4), // oklch(75.8% 0.033 293.6)
    node_shadow: Some(ShadowSpec {
        offset_y: 4.0,
        blur: 12.0,
        color: Color::rgba(0x140f2814),
    }),

    fg: Color::rgb(0x1a1822),        // oklch(21.6% 0.02 293.9)
    fg_muted: Color::rgb(0x5e5a6b),  // oklch(47.8% 0.027 296.1)
    fg_subtle: Color::rgb(0x9c97ab), // oklch(68.7% 0.03 297.1)

    accent: Color::rgb(0x6b3fc4),      // oklch(49.6% 0.195 293.1)
    accent_soft: Color::rgb(0x4a2890), // oklch(38.6% 0.161 292.1)
    accent_bg: Color::rgba(0x6b3fc41a),
    accent_line: Color::rgba(0x6b3fc466),
    active: None,
    row_selected_mix: [0.04, 0.05],

    green: Color::rgb(0x2f8a4d), // oklch(56.4% 0.127 150.9)
    green_soft: Color::rgba(0x2f8a4d24),
    yellow: Color::rgb(0xb88200), // oklch(64.4% 0.134 79.3)
    yellow_soft: Color::rgba(0xb8820024),
    blue: Color::rgb(0x3858c4), // oklch(50% 0.173 267.3)
    blue_soft: Color::rgba(0x3858c424),
    red: Color::rgb(0xc4353d), // oklch(54.9% 0.179 22.5)
    red_soft: Color::rgba(0xc4353d24),
    magenta: Color::rgb(0xa03a86), // oklch(45.6% 0.192 337.3)
    cyan: Color::rgb(0x0f6f77),    // oklch(45.1% 0.083 205.5)

    regions: [
        Color::rgb(0x7145b5), // oklch(50% 0.17 298)
        Color::rgb(0x3858c4),
        Color::rgb(0xb88200),
        Color::rgb(0x2f8a4d),
        Color::rgb(0xb24574), // oklch(55% 0.15 356)
    ],
    chart_series: [
        Color::rgb(0x6b3fc4),
        Color::rgb(0x3858c4),
        Color::rgb(0x2f8a4d),
        Color::rgb(0xb88200),
        Color::rgb(0xc4353d),
    ],
    node_types: NodeTypeColors {
        query: Color::rgb(0x6b3fc4),
        agent: Color::rgb(0x3858c4),
        result: Color::rgb(0x2f8a4d),
        chart: Color::rgb(0xb88200),
        error: Color::rgb(0xc4353d),
        variable: Color::rgb(0x855a69), // oklch(52% 0.06 357)
        activity: Color::rgb(0x0b7f80), // oklch(54% 0.09 196)
    },

    radius_node: 10.0,
    radius_card: 10.0,
    radius_pill: 999.0,
    node_frame: NodeFrame::Plain,
    type_indicator: TypeIndicator::Dot,

    // Rosé Pine Dawn (Monaco `rose-pine-dawn`).
    syntax: SyntaxSpec {
        keyword: Color::rgb(0x286983),
        keyword_control: Color::rgb(0x286983),
        string: Color::rgb(0xea9d34),
        number: Color::rgb(0xb4637a),
        function: Color::rgb(0x907aa9),
        type_name: Color::rgb(0x907aa9),
        variable: Color::rgb(0xd7827e),
        comment: Color::rgb(0x797593),
        operator: Color::rgb(0x797593),
    },
};
