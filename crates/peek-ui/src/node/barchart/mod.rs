//! The `BarChart` node: `~/labs/peek/src/canvas/nodes/BarChart/BarChartNode.tsx`.
//!
//! Rows are pushed in by the result node's `useChartSync`; this kind never queries.

mod columns;
mod plot;

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Entity, SharedString, Window, div, rems};
use peek_canvas::Document;
use peek_document::{BarChartData, ChartType, NodeId};
use peek_theme::ActivePeekTheme;

use super::kind::NodeContext;
use columns::ChartColumns;
use plot::{Series, SeriesChart};

/// What the body says while `useChartSync` has pushed no rows in.
const EMPTY_MESSAGE: &str = "No results";

/// The toggle's buttons, in `BarChartNode.tsx`'s order.
/// The three segments of `.chart-type-toggle`, each a glyph with its name as the tooltip.
const CHART_TYPES: [(ChartType, &str, IconName); 3] = [
    (ChartType::Bar, "Bar", IconName::ChartColumn),
    (ChartType::Line, "Line", IconName::ChartLine),
    (ChartType::Area, "Area", IconName::ChartArea),
];

/// `chartType` is optional on disk; `BarChartNode.tsx` reads a missing one as a bar chart.
fn chart_type(data: &BarChartData) -> ChartType {
    data.chart_type.unwrap_or(ChartType::Bar)
}

pub(crate) fn title(data: &BarChartData) -> String {
    if data.data.is_empty() {
        return "empty".to_string();
    }
    let columns = ChartColumns::of(data);
    format!("{} by {}", columns.primary_series(), columns.axis)
}

pub(crate) fn body(
    id: &NodeId,
    data: &BarChartData,
    context: NodeContext<'_>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.peek_theme();
    if data.data.is_empty() {
        return div()
            .id(ElementId::from(SharedString::from(format!("{id}-empty"))))
            .aria_label(EMPTY_MESSAGE)
            .test_support()
            .size_full()
            .pt(rems(0.875))
            .px(rems(1.0))
            .text_size(rems(0.75))
            .text_color(theme.fg_subtle)
            .child(EMPTY_MESSAGE)
            .into_any_element();
    }

    let columns = ChartColumns::of(data);
    let series = series(data, &columns, cx);
    let theme = cx.peek_theme();

    div()
        .v_flex()
        .size_full()
        .pt(rems(0.875))
        .px(rems(1.0))
        .pb(rems(0.75))
        .gap(rems(0.25))
        .child(
            div()
                .h_flex()
                .justify_between()
                .items_baseline()
                .gap(rems(0.5))
                .text_size(rems(0.875))
                .text_color(theme.fg)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_medium()
                        .child(columns.primary_series().to_string()),
                )
                .child(chart_type_toggle(id, data, context.document, cx)),
        )
        .child(
            div()
                .text_size(rems(0.6875))
                .text_color(theme.fg_muted)
                .mb(rems(0.5))
                .child(format!("by {} · {} points", columns.axis, data.data.len())),
        )
        .children(legend(&series, cx))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .pt(rems(0.5))
                .child(chart(id, data, &columns, series).rem_size(window.rem_size())),
        )
        .into_any_element()
}

pub(crate) fn header_extras(
    _id: &NodeId,
    _data: &BarChartData,
    _context: NodeContext<'_>,
    _window: &mut Window,
    _cx: &mut App,
) -> Option<AnyElement> {
    None
}

/// The bar / line / area segmented control, writing `chartType` straight to the document.
/// Each press is its own undo step, so `checkpoint` seals the edit immediately.
fn chart_type_toggle(
    id: &NodeId,
    data: &BarChartData,
    document: &Entity<Document>,
    cx: &App,
) -> Div {
    let theme = cx.peek_theme();
    let current = chart_type(data);
    div()
        .h_flex()
        .gap(rems(0.125))
        .p(rems(0.125))
        .rounded(rems(0.375))
        .bg(theme.node_bg_2)
        .border_1()
        .border_color(theme.node_border)
        .text_size(rems(0.625))
        .children(CHART_TYPES.map(|(next, label, icon)| {
            let selected = next == current;
            let (node, document) = (id.clone(), document.clone());
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{label}"))))
                .test_support()
                .h_flex()
                .items_center()
                .justify_center()
                .w(rems(1.5))
                .h(rems(1.375))
                .rounded(rems(0.25))
                .text_color(if selected { theme.fg } else { theme.fg_subtle })
                .when(selected, |button| button.bg(theme.accent_bg))
                .when(!selected, |button| {
                    let (background, ink) = (theme.node_bg, theme.fg);
                    button.hover(move |style| style.bg(background).text_color(ink))
                })
                .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
                .child(Icon::new(icon).size(rems(0.875)))
                .on_click(move |_, _, cx| {
                    document.update(cx, |document, cx| {
                        if document.update_data::<BarChartData>(&node, |data| {
                            data.chart_type = Some(next);
                        }) {
                            document.checkpoint();
                            cx.notify();
                        }
                    });
                })
        }))
}

/// One entry per numeric column, cycling the theme's five chart colours.
fn series(data: &BarChartData, columns: &ChartColumns, cx: &App) -> Vec<Series> {
    let palette = cx.peek_theme().chart_series;
    columns
        .series
        .iter()
        .enumerate()
        .map(|(index, name)| Series {
            name: SharedString::from(name.clone()),
            color: palette[index % palette.len()],
            values: columns::values(data, name),
        })
        .collect()
}

fn chart(
    id: &NodeId,
    data: &BarChartData,
    columns: &ChartColumns,
    series: Vec<Series>,
) -> SeriesChart {
    let labels = columns
        .labels(data)
        .into_iter()
        .map(SharedString::from)
        .collect();
    let element_id = ElementId::from(SharedString::from(format!("{id}-chart")));
    SeriesChart::new(element_id, labels, series).chart_type(chart_type(data))
}

/// With one series the title already names it; with more, colour alone would be the only
/// thing telling them apart.
fn legend(series: &[Series], cx: &App) -> Option<AnyElement> {
    if series.len() < 2 {
        return None;
    }
    let label = cx.peek_theme().fg_muted;
    Some(
        div()
            .h_flex()
            .flex_wrap()
            .gap_x(rems(0.75))
            .gap_y(rems(0.125))
            .text_size(rems(0.625))
            .text_color(label)
            .children(series.iter().map(|series| {
                div()
                    .h_flex()
                    .items_center()
                    .gap(rems(0.25))
                    .child(div().size(rems(0.4375)).rounded_full().bg(series.color))
                    .child(series.name.clone())
            }))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use columns::chart as chart_data;

    #[test]
    fn the_header_names_the_series_and_the_axis() {
        let data =
            chart_data(r#"[{"customer_name":"Vicosight","total_quotes":142,"signed_quotes":12}]"#);
        assert_eq!(title(&data), "total_quotes by customer_name");
    }

    #[test]
    fn a_chart_with_no_rows_is_headed_empty() {
        assert_eq!(title(&chart_data("[]")), "empty");
    }
}
