//! Projects a [`ThemeSpec`] onto gpui-component's `ThemeConfig` so buttons, dialogs, lists and
//! the palette pick up Peek's colours. Keys left unset fall back along gpui-component's own
//! chain (popover ← background, table ← list, …), so only roles with a Peek meaning are set.

use std::rc::Rc;

use gpui_kit::SharedString;
use gpui_kit::component::highlighter::HighlightThemeStyle;
use gpui_kit::component::theme::{ThemeConfig, ThemeConfigColors, ThemeMode};
use serde_json::{Map, Value};

use crate::spec::{BLACK, Color, ThemeSpec, WHITE};

/// The base font shared by UI and code, as the CSS did.
pub(crate) const FONT_FAMILY: &str = "Monaspace Krypton";
pub(crate) const FONT_SIZE: f32 = 13.0;

#[must_use]
pub fn to_component_config(spec: &ThemeSpec) -> Rc<ThemeConfig> {
    let mut entries = color_entries(spec);
    entries.extend(palette_entries(spec));
    let colors = colors_from_entries(&entries);

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "radii are small non-negative pixel counts"
    )]
    let radius = |value: f32| Some(value.round().max(0.0) as usize);

    Rc::new(ThemeConfig {
        is_default: false,
        name: SharedString::from(spec.name),
        mode: if spec.is_light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        font_size: Some(FONT_SIZE),
        font_family: Some(SharedString::from(FONT_FAMILY)),
        mono_font_family: Some(SharedString::from(FONT_FAMILY)),
        mono_font_size: Some(FONT_SIZE),
        radius: radius(spec.radius_card),
        radius_lg: radius(spec.radius_node),
        shadow: Some(spec.node_shadow.is_some()),
        colors,
        highlight: Some(highlight_from_spec(spec)),
    })
}

/// The SQL editor's colours: the code itself, plus the chrome around it.
///
/// `ThemeStyle`'s fields are private for the same reason `ThemeConfigColors`' are, so this
/// goes through the dotted keys a Zed theme file would use, exactly as [`colors_from_entries`]
/// does below.
///
/// # Panics
/// Never: every key here is a field of `HighlightThemeStyle` and every value a valid hex string.
fn highlight_from_spec(spec: &ThemeSpec) -> HighlightThemeStyle {
    let syntax: Map<String, Value> = syntax_entries(spec)
        .into_iter()
        .map(|(role, color)| (role.to_string(), style_value(color)))
        .collect();

    let mut map: Map<String, Value> = [
        // The editor sits inside a node card, so it takes the card's surface rather than the
        // canvas background — otherwise it reads as a hole cut in the node.
        ("editor.background", spec.node_bg),
        ("editor.foreground", spec.fg),
        ("editor.gutter.background", spec.node_bg),
        ("editor.line_number", spec.fg_subtle),
        ("editor.active_line_number", spec.fg_muted),
        ("editor.active_line.background", spec.node_inset),
        ("editor.invisible", spec.fg_subtle),
        // `StatusColors` is flattened into the same object; these paint the squiggles.
        ("error", spec.red),
        ("warning", spec.yellow),
        ("info", spec.blue),
        ("hint", spec.fg_muted),
        ("success", spec.green),
    ]
    .into_iter()
    .map(|(key, color)| (key.to_string(), Value::String(color.hex())))
    .collect();
    map.insert("syntax".to_string(), Value::Object(syntax));

    serde_json::from_value(Value::Object(map)).expect("highlight keys match HighlightThemeStyle")
}

/// gpui-component's syntax roles, filled from Peek's nine `SyntaxSpec` colours.
///
/// Roles carry more than their name suggests, because `SyntaxColors::style` falls back along
/// the dots: `@type.builtin` and `@type.qualifier` both land on `type`, `@function.call` on
/// `function`, `@punctuation.bracket` and `@punctuation.delimiter` on `punctuation`.
///
/// `constant` is a carrier, not a literal: SQL emits no `@constant`, so `sql_highlights`
/// remaps `@keyword.operator` (`AND`, `OR`, `IN`, `NOT`) onto it to keep Peek's second keyword
/// hue, which would otherwise fall back to plain `keyword`. Change one site and change both.
fn syntax_entries(spec: &ThemeSpec) -> Vec<(&'static str, Color)> {
    let syntax = spec.syntax;
    vec![
        ("keyword", syntax.keyword),
        ("constant", syntax.keyword_control),
        ("string", syntax.string),
        ("number", syntax.number),
        ("boolean", syntax.number),
        ("function", syntax.function),
        ("type", syntax.type_name),
        ("variable", syntax.variable),
        ("attribute", syntax.variable),
        ("comment", syntax.comment),
        ("operator", syntax.operator),
        ("punctuation", syntax.operator),
    ]
}

fn style_value(color: Color) -> Value {
    let mut style = Map::new();
    style.insert("color".to_string(), Value::String(color.hex()));
    Value::Object(style)
}

/// `ThemeConfigColors` keeps its `base.*` fields private and is meant to be read from JSON, so
/// the mapping goes through the same dotted keys a theme file would use.
///
/// # Panics
/// Never: every key here is a field of `ThemeConfigColors` and every value a valid hex string.
fn colors_from_entries(entries: &[(&str, Color)]) -> ThemeConfigColors {
    let map: Map<String, Value> = entries
        .iter()
        .map(|(key, color)| ((*key).to_string(), Value::String(color.hex())))
        .collect();
    serde_json::from_value(Value::Object(map)).expect("theme colour keys match ThemeConfigColors")
}

/// gpui-component roles derived from Peek's tokens, keyed the way a theme file spells them.
fn color_entries(spec: &ThemeSpec) -> Vec<(&'static str, Color)> {
    let on_accent = if spec.is_light { spec.node_bg } else { spec.bg };
    let (row_selected, row_selected_hover) = spec.row_selected();
    let selection = spec.selection();
    vec![
        ("background", spec.bg),
        ("foreground", spec.fg),
        ("border", spec.node_border),
        ("input.border", spec.node_border_strong),
        ("ring", selection),
        ("caret", selection),
        ("selection.background", spec.accent.with_alpha(0.3)),
        ("muted.background", spec.node_inset),
        ("muted.foreground", spec.fg_muted),
        ("primary.background", spec.accent),
        ("primary.foreground", on_accent),
        ("primary.hover.background", spec.accent.mix(spec.fg, 0.08)),
        ("primary.active.background", spec.accent.mix(spec.bg, 0.14)),
        ("secondary.background", spec.node_bg_2),
        ("secondary.foreground", spec.fg),
        ("secondary.hover.background", row_selected),
        ("secondary.active.background", row_selected_hover),
        ("accent.background", spec.accent_bg),
        ("accent.foreground", spec.fg),
        ("popover.background", spec.node_bg_2),
        ("popover.foreground", spec.fg),
        ("list.background", spec.node_bg),
        ("list.active.background", spec.accent_bg),
        ("list.active.border", spec.accent_line),
        ("list.hover.background", row_selected),
        ("list.even.background", spec.node_bg),
        ("list.head.background", spec.node_bg_2),
        ("table.background", spec.node_bg),
        // `Result.css` puts the head on the node's own background, not a raised one: the column
        // names read as labels over the rows rather than as a separate bar.
        ("table.head.background", spec.node_bg),
        ("table.head.foreground", spec.fg_muted),
        ("table.row.border", spec.node_border),
        // Hover is `--pk-node-bg-2` and selection is `--pk-row-selected-bg`. They used to be the
        // same colour here, which left a selected row indistinguishable from a hovered one.
        ("table.hover.background", spec.node_bg_2),
        ("table.active.background", spec.accent_bg),
        ("table.active.border", spec.accent_line),
        ("table.even.background", spec.node_bg),
        ("link", spec.accent),
        ("scrollbar.background", spec.bg.with_alpha(0.0)),
        (
            "scrollbar.thumb.background",
            spec.fg_subtle.with_alpha(0.45),
        ),
        (
            "scrollbar.thumb.hover.background",
            spec.fg_subtle.with_alpha(0.7),
        ),
        ("skeleton.background", spec.node_bg_2),
        ("progress.bar.background", spec.accent),
        ("slider.background", spec.accent),
        ("slider.thumb.background", spec.fg),
        ("switch.background", spec.node_border_strong),
        ("switch.thumb.background", spec.fg),
        ("tab.background", spec.node_bg),
        ("tab_bar.background", spec.bg),
        ("tab.active.background", spec.node_bg_2),
        ("tab.foreground", spec.fg_muted),
        ("tab.active.foreground", spec.fg),
        ("tab_bar.segmented.background", spec.node_inset),
        ("title_bar.background", spec.bg),
        ("title_bar.border", spec.node_border),
        ("status_bar.background", spec.bg),
        ("status_bar.border", spec.node_border),
        ("window.border", spec.node_border_strong),
        ("drag.border", spec.accent.with_alpha(0.65)),
        ("drop_target.background", spec.accent_bg),
        ("danger.background", spec.red),
        ("danger.foreground", on_accent),
        ("success.background", spec.green),
        ("success.foreground", on_accent),
        ("warning.background", spec.yellow),
        ("warning.foreground", on_accent),
        ("info.background", spec.blue),
        ("info.foreground", on_accent),
        ("button.background", spec.node_bg_2),
        ("button.hover.background", row_selected),
        ("button.foreground", spec.fg),
    ]
}

/// Chart series and the base palette gpui-component derives its light variants from.
fn palette_entries(spec: &ThemeSpec) -> Vec<(&'static str, Color)> {
    let towards = if spec.is_light { BLACK } else { WHITE };
    vec![
        ("chart.1", spec.chart_series[0]),
        ("chart.2", spec.chart_series[1]),
        ("chart.3", spec.chart_series[2]),
        ("chart.4", spec.chart_series[3]),
        ("chart.5", spec.chart_series[4]),
        ("chart_bullish", spec.green),
        ("chart_bearish", spec.red),
        ("base.red", spec.red),
        ("base.red.light", spec.red.mix(towards, 0.3)),
        ("base.green", spec.green),
        ("base.green.light", spec.green.mix(towards, 0.3)),
        ("base.blue", spec.blue),
        ("base.blue.light", spec.blue.mix(towards, 0.3)),
        ("base.yellow", spec.yellow),
        ("base.yellow.light", spec.yellow.mix(towards, 0.3)),
        ("base.magenta", spec.node_types.variable),
        (
            "base.magenta.light",
            spec.node_types.variable.mix(towards, 0.3),
        ),
        ("base.cyan", spec.node_types.activity),
        (
            "base.cyan.light",
            spec.node_types.activity.mix(towards, 0.3),
        ),
    ]
}
