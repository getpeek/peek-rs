//! The theme as the UI consumes it: gpui colours and pixel values, resolved once per switch.

use gpui_kit::{App, Global, Hsla, Pixels, SharedString, px, rgba};
use peek_config::ThemeId;
use peek_document::NodeType;

use crate::spec::{Color, NodeFrame, ThemeSpec, TypeIndicator};

impl From<Color> for Hsla {
    fn from(color: Color) -> Self {
        rgba(color.packed()).into()
    }
}

/// Resting edges are muted so they read as quiet background structure (`node.css`).
const REST_OPACITY: f32 = 0.7;
/// `.connection-active` brightens an edge touching a selected node.
const ACTIVE_WHITE_MIX: f32 = 0.38;

/// How an edge is drawn right now (`node.css`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeState {
    Resting,
    /// The edge itself is selected: full opacity, same stroke.
    Selected,
    /// The edge's source or target node is selected (`useSelectionHighlight.ts`): brighter and
    /// thicker. It wins over [`EdgeState::Selected`], as its later `node.css` rule does.
    ConnectionActive,
}

/// `color-mix(in oklab, colour, white 38%)`, close enough in HSL: lift the lightness the rest
/// of the way to white and bleed the saturation by the same fraction.
fn toward_white(color: Hsla, amount: f32) -> Hsla {
    Hsla {
        s: color.s * (1.0 - amount),
        l: color.l + (1.0 - color.l) * amount,
        ..color
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedFrame {
    Plain,
    Brackets {
        color: Hsla,
        length: f32,
        thickness: f32,
        selected_length: f32,
        selected_thickness: f32,
    },
}

#[derive(Debug, Clone)]
pub struct PeekTheme {
    pub id: ThemeId,
    pub name: SharedString,
    pub is_light: bool,

    pub bg: Hsla,
    pub bg_grid: Hsla,
    pub canvas_base: Hsla,
    pub canvas_gradient: Option<(Hsla, Hsla)>,
    pub node_bg: Hsla,
    pub node_bg_2: Hsla,
    pub node_inset: Hsla,
    pub node_border: Hsla,
    pub node_border_strong: Hsla,
    pub node_shadow: Option<(Pixels, Pixels, Hsla)>,
    /// The floating canvas panels (toolbar, zoom cluster): a half-opaque [`Self::node_bg`] behind
    /// a [`Self::node_border`] hairline, and a shadow deeper than a node's. `Toolbar.css`.
    pub chrome_bg: Hsla,
    pub chrome_shadow: Option<(Pixels, Pixels, Hsla)>,

    pub fg: Hsla,
    pub fg_muted: Hsla,
    pub fg_subtle: Hsla,

    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_bg: Hsla,
    pub accent_line: Hsla,
    /// Selection ring and marquee colour (`active` when the theme has one, else the accent).
    pub selection: Hsla,
    pub row_selected_bg: Hsla,
    pub row_selected_bg_hover: Hsla,

    pub green: Hsla,
    pub green_soft: Hsla,
    pub yellow: Hsla,
    pub yellow_soft: Hsla,
    pub blue: Hsla,
    pub blue_soft: Hsla,
    pub red: Hsla,
    pub red_soft: Hsla,
    pub magenta: Hsla,
    pub cyan: Hsla,

    pub regions: [Hsla; 5],
    pub chart_series: [Hsla; 5],
    node_types: [Hsla; 7],

    pub radius_node: Pixels,
    pub radius_card: Pixels,
    pub radius_pill: Pixels,
    pub node_frame: ResolvedFrame,
    pub type_indicator: TypeIndicator,
}

impl PeekTheme {
    #[must_use]
    pub fn from_spec(spec: &ThemeSpec) -> Self {
        let (row_selected_bg, row_selected_bg_hover) = spec.row_selected();
        let kinds = spec.node_types;
        Self {
            id: spec.id,
            name: SharedString::from(spec.name),
            is_light: spec.is_light,
            bg: spec.bg.into(),
            bg_grid: spec.bg_grid.into(),
            canvas_base: spec.canvas.base.into(),
            canvas_gradient: spec
                .canvas
                .gradient
                .map(|(top, bottom)| (top.into(), bottom.into())),
            node_bg: spec.node_bg.into(),
            node_bg_2: spec.node_bg_2.into(),
            node_inset: spec.node_inset.into(),
            node_border: spec.node_border.into(),
            node_border_strong: spec.node_border_strong.into(),
            node_shadow: spec
                .node_shadow
                .map(|shadow| (px(shadow.offset_y), px(shadow.blur), shadow.color.into())),
            chrome_bg: spec.chrome().into(),
            // The reference hardcodes `0 12px 32px rgba(0,0,0,.4)`; the offsets are the spec but
            // the tint comes from the theme, so a flat theme stays flat and a light one stays light.
            chrome_shadow: spec
                .node_shadow
                .map(|shadow| (px(12.0), px(32.0), shadow.color.into())),
            fg: spec.fg.into(),
            fg_muted: spec.fg_muted.into(),
            fg_subtle: spec.fg_subtle.into(),
            accent: spec.accent.into(),
            accent_soft: spec.accent_soft.into(),
            accent_bg: spec.accent_bg.into(),
            accent_line: spec.accent_line.into(),
            selection: spec.selection().into(),
            row_selected_bg: row_selected_bg.into(),
            row_selected_bg_hover: row_selected_bg_hover.into(),
            green: spec.green.into(),
            green_soft: spec.green_soft.into(),
            yellow: spec.yellow.into(),
            yellow_soft: spec.yellow_soft.into(),
            blue: spec.blue.into(),
            blue_soft: spec.blue_soft.into(),
            red: spec.red.into(),
            red_soft: spec.red_soft.into(),
            magenta: spec.magenta.into(),
            cyan: spec.cyan.into(),
            regions: spec.regions.map(Into::into),
            chart_series: spec.chart_series.map(Into::into),
            node_types: [
                kinds.query.into(),
                kinds.agent.into(),
                kinds.result.into(),
                kinds.chart.into(),
                kinds.error.into(),
                kinds.variable.into(),
                kinds.activity.into(),
            ],
            radius_node: px(spec.radius_node),
            radius_card: px(spec.radius_card),
            radius_pill: px(spec.radius_pill),
            node_frame: match spec.node_frame {
                NodeFrame::Plain => ResolvedFrame::Plain,
                NodeFrame::Brackets {
                    color,
                    length,
                    thickness,
                    selected_length,
                    selected_thickness,
                } => ResolvedFrame::Brackets {
                    color: color.into(),
                    length,
                    thickness,
                    selected_length,
                    selected_thickness,
                },
            },
            type_indicator: spec.type_indicator,
        }
    }

    /// The `--pk-type-*` accent for a node kind, or `None` for the kinds `nodeTypeColor.ts`
    /// leaves out. The two callers disagree about the fallback, so neither is baked in here.
    fn node_type_accent(&self, kind: Option<NodeType>) -> Option<Hsla> {
        match kind {
            Some(NodeType::Query) => Some(self.node_types[0]),
            Some(NodeType::Agent) => Some(self.node_types[1]),
            Some(NodeType::Result | NodeType::ResultInsertForm) => Some(self.node_types[2]),
            Some(NodeType::Barchart) => Some(self.node_types[3]),
            Some(NodeType::QueryError) => Some(self.node_types[4]),
            Some(NodeType::Variable) => Some(self.node_types[5]),
            Some(NodeType::Activity) => Some(self.node_types[6]),
            Some(NodeType::Text | NodeType::Draw | NodeType::TableDefinition) | None => None,
        }
    }

    /// The accent for a node kind (`nodeTypeColor.ts`); kinds without one use the muted text.
    #[must_use]
    pub fn node_type(&self, kind: Option<NodeType>) -> Hsla {
        self.node_type_accent(kind).unwrap_or(self.fg_muted)
    }

    /// An edge's stroke, tinted by its **target** kind — "what this feeds", so the kind an edge
    /// points at is readable at a glance (`FloatingEdge.tsx`). Alpha is already applied.
    ///
    /// `node.css` writes the tint into `--pk-edge-color`, so a *selected* edge keeps that tint
    /// and only the untinted kinds fall through to the accent.
    #[must_use]
    pub fn edge(&self, target: Option<NodeType>, state: EdgeState) -> Hsla {
        let tint = self.node_type_accent(target);
        match state {
            EdgeState::Resting => tint
                .unwrap_or(self.node_border_strong)
                .opacity(REST_OPACITY),
            EdgeState::ConnectionActive => {
                toward_white(tint.unwrap_or(self.accent), ACTIVE_WHITE_MIX)
            }
            EdgeState::Selected => tint.unwrap_or(self.accent),
        }
    }

    /// Region palette entry for a region's `colorIndex`, wrapping like the CSS did.
    #[must_use]
    pub fn region(&self, color_index: usize) -> Hsla {
        self.regions[color_index % self.regions.len()]
    }
}

impl Global for PeekTheme {}

/// `cx.peek_theme()` beside gpui-component's `cx.theme()`.
pub trait ActivePeekTheme {
    fn peek_theme(&self) -> &PeekTheme;
}

impl ActivePeekTheme for App {
    fn peek_theme(&self) -> &PeekTheme {
        self.global::<PeekTheme>()
    }
}

#[cfg(test)]
mod tests {
    use peek_config::ThemeId;
    use peek_document::NodeType;

    use super::{EdgeState, PeekTheme, REST_OPACITY};
    use crate::builtin;

    fn theme() -> PeekTheme {
        PeekTheme::from_spec(builtin::spec(ThemeId::Midday))
    }

    #[test]
    fn an_edge_is_tinted_by_the_kind_it_points_at() {
        let theme = theme();

        assert_eq!(
            theme.edge(Some(NodeType::Query), EdgeState::Selected),
            theme.node_type(Some(NodeType::Query)),
            "a selected edge keeps its target's tint"
        );
        assert_ne!(
            theme.edge(Some(NodeType::Query), EdgeState::Resting),
            theme.edge(Some(NodeType::Result), EdgeState::Resting),
            "different targets read differently"
        );
    }

    #[test]
    fn an_untinted_target_falls_back_to_the_border_not_the_muted_text() {
        let theme = theme();

        let resting = theme.edge(Some(NodeType::Text), EdgeState::Resting);
        assert_eq!(
            resting,
            theme.node_border_strong.opacity(REST_OPACITY),
            "`node.css` falls back to --pk-node-border-strong, not --pk-fg-muted"
        );
        assert_eq!(
            theme.edge(Some(NodeType::Text), EdgeState::Selected),
            theme.accent,
            "and only an untinted edge turns accent when selected"
        );
    }

    #[test]
    fn resting_edges_are_dimmed_and_active_ones_are_not() {
        let theme = theme();

        assert!(
            theme.edge(Some(NodeType::Query), EdgeState::Resting).a < 1.0,
            "resting edges read as quiet background structure"
        );
        let active = theme.edge(Some(NodeType::Query), EdgeState::ConnectionActive);
        let resting = theme.edge(Some(NodeType::Query), EdgeState::Resting);
        assert!(
            active.a > resting.a && active.l > resting.l,
            "and .connection-active is brighter"
        );
    }
}
