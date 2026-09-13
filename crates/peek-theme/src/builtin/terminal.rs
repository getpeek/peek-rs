#![allow(
    clippy::unreadable_literal,
    reason = "colour literals are hex triplets, which separators would obscure"
)]

use peek_config::ThemeId;

use crate::spec::{
    CanvasBackground, Color, NodeFrame, NodeTypeColors, SyntaxSpec, ThemeSpec, TypeIndicator,
};

const WHITE: Color = Color::rgb(0xffffff);
const PHOSPHOR: Color = Color::rgb(0x33ff66);

/// Tactical command console: pure black, zero radius, corner brackets, phosphor selection.
pub(super) static TERMINAL: ThemeSpec = ThemeSpec {
    id: ThemeId::Terminal,
    name: "Terminal",
    tagline: "Command console",
    is_light: false,

    bg: Color::rgb(0x000000),
    bg_grid: Color::rgba(0xffffff1f),
    canvas: CanvasBackground {
        base: Color::rgb(0x000000),
        gradient: None,
    },
    node_bg: Color::rgb(0x000000),
    node_bg_2: Color::rgb(0x111111),
    node_inset: Color::rgb(0x0a0a0a),
    node_border: Color::rgb(0x262626),
    node_border_strong: Color::rgb(0x4a4a4a),
    node_shadow: None,

    fg: WHITE,
    fg_muted: Color::rgb(0xb5b5b5),
    fg_subtle: Color::rgb(0x6e6e6e),

    accent: WHITE,
    accent_soft: Color::rgb(0xe6e6e6),
    accent_bg: Color::rgba(0xffffff1f),
    accent_line: Color::rgba(0xffffff8c),
    active: Some(PHOSPHOR),
    row_selected_mix: [0.09, 0.14],

    green: WHITE,
    green_soft: Color::rgba(0xffffff24),
    yellow: Color::rgb(0xffd633),
    yellow_soft: Color::rgba(0xffd63329),
    blue: Color::rgb(0x33ccff),
    blue_soft: Color::rgba(0x33ccff29),
    red: Color::rgb(0xff5c57),
    red_soft: Color::rgba(0xff5c5729),
    magenta: Color::rgb(0xff5cff),
    cyan: Color::rgb(0x33ffd6),

    // The CSS had no region palette for Terminal; grayscale keeps the console look.
    regions: [
        WHITE,
        Color::rgb(0x33ccff),
        Color::rgb(0xffd633),
        Color::rgb(0xb5b5b5),
        Color::rgb(0xc77dff),
    ],
    // Phosphor ramp, brightest series first (`.react-flow__node-barchart` override).
    chart_series: [
        PHOSPHOR,
        Color::rgb(0x2bcc52),
        Color::rgb(0x7de89a),
        Color::rgb(0x4a9960),
        Color::rgb(0xb8b8b8),
    ],
    node_types: NodeTypeColors {
        query: WHITE,
        agent: Color::rgb(0x33ccff),
        result: Color::rgb(0xffd633),
        chart: Color::rgb(0xff9e33),
        error: Color::rgb(0xff5c57),
        variable: Color::rgb(0xc77dff),
        activity: Color::rgb(0x33e0d0),
    },

    radius_node: 0.0,
    radius_card: 0.0,
    radius_pill: 999.0,
    node_frame: NodeFrame::Brackets {
        color: Color::rgb(0xd0d0d0),
        length: 13.0,
        thickness: 1.0,
        selected_length: 16.0,
        selected_thickness: 2.0,
    },
    type_indicator: TypeIndicator::Tick,

    syntax: SyntaxSpec {
        keyword: PHOSPHOR,
        keyword_control: PHOSPHOR,
        string: Color::rgb(0xffd633),
        number: Color::rgb(0xff9e33),
        function: Color::rgb(0x56d4ff),
        type_name: Color::rgb(0x56d4ff),
        variable: Color::rgb(0xc77dff),
        comment: Color::rgb(0x6e6e6e),
        operator: Color::rgb(0xb5b5b5),
    },
};
