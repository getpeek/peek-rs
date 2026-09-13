#![allow(
    clippy::unreadable_literal,
    reason = "colour literals are hex triplets, which separators would obscure"
)]

use peek_config::ThemeId;

use crate::spec::{
    CanvasBackground, Color, NodeFrame, NodeTypeColors, ShadowSpec, SyntaxSpec, ThemeSpec,
    TypeIndicator,
};

const CYAN: Color = Color::rgb(0x9ccfd8);

/// Cold technical drafting theme on the Rosé Pine Moon palette.
pub(super) static BLUEPRINT: ThemeSpec = ThemeSpec {
    id: ThemeId::Blueprint,
    name: "Blueprint",
    tagline: "Drafting sheet",
    is_light: false,

    bg: Color::rgb(0x232136),
    bg_grid: Color::rgb(0x44415a),
    canvas: CanvasBackground {
        base: Color::rgb(0x232136),
        // The CSS top vignette, flattened to a vertical gradient.
        gradient: Some((Color::rgb(0x2a2740), Color::rgb(0x232136))),
    },
    node_bg: Color::rgb(0x2a273f),
    node_bg_2: Color::rgb(0x393552),
    node_inset: Color::rgb(0x201e30),
    node_border: Color::rgb(0x56526e),
    node_border_strong: Color::rgb(0x6e6a86),
    node_shadow: Some(ShadowSpec {
        offset_y: 12.0,
        blur: 32.0,
        color: Color::rgba(0x00000080),
    }),

    fg: Color::rgb(0xe0def4),
    fg_muted: Color::rgb(0x908caa),
    fg_subtle: Color::rgb(0x6e6a86),

    accent: CYAN,
    accent_soft: Color::rgb(0xcfe9ee),
    accent_bg: Color::rgba(0x9ccfd824),
    accent_line: Color::rgba(0x9ccfd88c),
    active: None,
    row_selected_mix: [0.06, 0.10],

    green: CYAN,
    green_soft: Color::rgba(0x9ccfd829),
    yellow: Color::rgb(0xf6c177),
    yellow_soft: Color::rgba(0xf6c17729),
    blue: Color::rgb(0x3e8fb0),
    blue_soft: Color::rgba(0x3e8fb029),
    red: Color::rgb(0xeb6f92),
    red_soft: Color::rgba(0xeb6f9229),
    magenta: Color::rgb(0xc4a7e7), // iris
    cyan: CYAN,                    // foam, the same hue as the accent

    regions: [
        Color::rgb(0xc4a7e7),
        Color::rgb(0x3e8fb0),
        Color::rgb(0xf6c177),
        CYAN,
        Color::rgb(0xea9a97),
    ],
    chart_series: [
        CYAN,
        Color::rgb(0x3e8fb0),
        Color::rgb(0xc4a7e7),
        Color::rgb(0xf6c177),
        Color::rgb(0xeb6f92),
    ],
    node_types: NodeTypeColors {
        query: CYAN,
        agent: Color::rgb(0xc4a7e7),
        result: Color::rgb(0x3e8fb0),
        chart: Color::rgb(0xf6c177),
        error: Color::rgb(0xeb6f92),
        variable: Color::rgb(0xea9a97),
        activity: Color::rgb(0x6fd6cf),
    },

    radius_node: 2.0,
    radius_card: 2.0,
    radius_pill: 2.0,
    node_frame: NodeFrame::Brackets {
        color: CYAN,
        length: 17.0,
        thickness: 2.0,
        selected_length: 22.0,
        selected_thickness: 2.0,
    },
    type_indicator: TypeIndicator::Tick,

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
