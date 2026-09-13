#![allow(
    clippy::unreadable_literal,
    reason = "colour literals are hex triplets, which separators would obscure"
)]

use peek_config::ThemeId;

use crate::spec::{
    CanvasBackground, Color, NodeFrame, NodeTypeColors, ShadowSpec, SyntaxSpec, ThemeSpec,
    TypeIndicator,
};

/// Warm editorial light theme on the Rosé Pine Dawn palette.
pub(super) static PAPER: ThemeSpec = ThemeSpec {
    id: ThemeId::Paper,
    name: "Paper",
    tagline: "Warm editorial light",
    is_light: true,

    bg: Color::rgb(0xfaf4ed),
    bg_grid: Color::rgb(0xdfdad9),
    canvas: CanvasBackground {
        base: Color::rgb(0xfaf4ed),
        gradient: Some((Color::rgb(0xfffaf3), Color::rgb(0xfaf4ed))),
    },
    node_bg: Color::rgb(0xfffaf3),
    node_bg_2: Color::rgb(0xf2e9e1),
    node_inset: Color::rgb(0xede4dc),
    node_border: Color::rgb(0xcecacd),
    node_border_strong: Color::rgb(0x9893a5),
    node_shadow: Some(ShadowSpec {
        offset_y: 12.0,
        blur: 30.0,
        color: Color::rgba(0x5752791a),
    }),

    fg: Color::rgb(0x575279),
    fg_muted: Color::rgb(0x797593),
    fg_subtle: Color::rgb(0x9893a5),

    accent: Color::rgb(0xd7827e),
    accent_soft: Color::rgb(0xf0c9c7),
    accent_bg: Color::rgba(0xd7827e24),
    accent_line: Color::rgba(0xd7827e73),
    active: None,
    row_selected_mix: [0.04, 0.06],

    green: Color::rgb(0x56949f),
    green_soft: Color::rgba(0x56949f29),
    yellow: Color::rgb(0xea9d34),
    yellow_soft: Color::rgba(0xea9d3429),
    blue: Color::rgb(0x286983),
    blue_soft: Color::rgba(0x28698324),
    red: Color::rgb(0xb4637a),
    red_soft: Color::rgba(0xb4637a24),
    magenta: Color::rgb(0x907aa9), // iris
    cyan: Color::rgb(0x2f7d84),    // foam, darkened for cream

    regions: [
        Color::rgb(0x907aa9),
        Color::rgb(0x286983),
        Color::rgb(0xea9d34),
        Color::rgb(0x56949f),
        Color::rgb(0xd7827e),
    ],
    chart_series: [
        Color::rgb(0xd7827e),
        Color::rgb(0x286983),
        Color::rgb(0x56949f),
        Color::rgb(0xea9d34),
        Color::rgb(0xb4637a),
    ],
    node_types: NodeTypeColors {
        query: Color::rgb(0xb4637a),
        agent: Color::rgb(0x907aa9),
        result: Color::rgb(0x56949f),
        chart: Color::rgb(0xea9d34),
        error: Color::rgb(0xb4637a),
        variable: Color::rgb(0x286983),
        activity: Color::rgb(0x3f8f96),
    },

    radius_node: 14.0,
    radius_card: 12.0,
    radius_pill: 999.0,
    node_frame: NodeFrame::Plain,
    type_indicator: TypeIndicator::Dot,

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
