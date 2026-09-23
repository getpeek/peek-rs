//! The chart itself: one [`Plot`] that draws bars, lines or areas for any number of series.
//!
//! gpui-component ships `BarChart`, `LineChart` and `AreaChart`, but each takes a single
//! value accessor and derives its own y-domain, so stacking one per series would silently
//! rescale every series against its own maximum. Peek's charts are routinely multi-series
//! (`total_quotes` beside `signed_quotes`), so the series share one domain here and the
//! drawing is composed from the public plot primitives instead.

use gpui_kit::component::plot::label::{
    PlotLabel, TEXT_GAP, TEXT_SIZE, Text, truncate_text_to_width,
};
use gpui_kit::component::plot::scale::{Scale, ScaleBand, ScaleLinear, ScalePoint};
use gpui_kit::component::plot::shape::{Area, Bar, BarAlignment, Line};
use gpui_kit::component::plot::tooltip::{CrossLine, Dot, Tooltip, TooltipState};
use gpui_kit::component::plot::{AXIS_GAP, IntoPlot, Plot, PlotAxis, StrokeStyle};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, Corners, ElementId, Hsla, Pixels, Point, SharedString, Size,
    TextAlign, Window, point, px,
};
use peek_document::ChartType;
use peek_theme::ActivePeekTheme;

use crate::node::BASE_REM;

/// Width reserved left of the plot for the value-axis labels, and height reserved below it
/// for the category labels. Every length here is designed in pixels at zoom 1 and multiplied
/// by [`SeriesChart::scale`] when painted: a plot paints straight to the window, outside the
/// rem scope that scales the rest of the node with the camera.
const VALUE_AXIS_WIDTH: f32 = 34.0;
const TOP_PADDING: f32 = 8.0;
/// Horizontal room one category label needs before its neighbour has to be dropped.
const LABEL_BUDGET: f32 = 44.0;
/// Surface gap between the bars of one category, so adjacent fills never touch.
const BAR_GAP: f32 = 2.0;
const BAR_CORNER: f32 = 4.0;
const LINE_WIDTH: f32 = 2.0;
const DOT_SIZE: f32 = 6.0;
const AREA_OPACITY: f32 = 0.18;
/// Intervals the value axis is divided into; five labels, zero included.
const VALUE_INTERVALS: u16 = 4;

#[derive(Debug, Clone)]
pub(crate) struct Series {
    pub(crate) name: SharedString,
    pub(crate) color: Hsla,
    /// One entry per category; `None` where the row carried no number.
    pub(crate) values: Vec<Option<f64>>,
}

#[derive(Debug, IntoPlot)]
pub(crate) struct SeriesChart {
    id: ElementId,
    labels: Vec<SharedString>,
    series: Vec<Series>,
    chart_type: ChartType,
    /// Screen pixels per design pixel: the node's rem scope over the rem it was designed at.
    scale: f32,
}

impl SeriesChart {
    pub(crate) fn new(id: ElementId, labels: Vec<SharedString>, series: Vec<Series>) -> Self {
        Self {
            id,
            labels,
            series,
            chart_type: ChartType::Bar,
            scale: 1.0,
        }
    }

    #[must_use]
    pub(crate) fn chart_type(mut self, chart_type: ChartType) -> Self {
        self.chart_type = chart_type;
        self
    }

    /// The rem scope the node is laid out in, so the plot zooms with the rest of the node.
    #[must_use]
    pub(crate) fn rem_size(mut self, rem_size: Pixels) -> Self {
        self.scale = rem_size.as_f32() / BASE_REM;
        self
    }

    /// A length designed at zoom 1, as the camera draws it.
    fn scaled(&self, design: f32) -> f32 {
        design * self.scale
    }

    /// The plot area inside `bounds`, once the axis labels have taken their room.
    fn frame(&self, bounds: Bounds<Pixels>) -> Frame {
        let (left, top) = (self.scaled(VALUE_AXIS_WIDTH), self.scaled(TOP_PADDING));
        Frame {
            left,
            top,
            bottom: (bounds.size.height.as_f32() - self.scaled(AXIS_GAP)).max(top),
            width: (bounds.size.width.as_f32() - left).max(0.0),
        }
    }

    /// The value range every series shares — the whole point of drawing the series here
    /// rather than stacking gpui-component's single-series charts. Zero is always in it, so
    /// bars grow from a baseline rather than from the smallest value present, and a flat
    /// dataset gets one unit of headroom: a zero-width domain scales to nothing at all.
    fn extent(&self) -> (f64, f64) {
        let (low, high) = self
            .series
            .iter()
            .flat_map(|series| series.values.iter().flatten())
            .fold((0.0_f64, 0.0_f64), |(low, high), value| {
                (low.min(*value), high.max(*value))
            });
        if (high - low).abs() < f64::EPSILON {
            return (low, high + 1.0);
        }
        (low, high)
    }

    fn value_scale(&self, frame: Frame) -> ScaleLinear<f64> {
        let (low, high) = self.extent();
        ScaleLinear::new(vec![low, high], vec![frame.bottom, frame.top])
    }

    /// Horizontal centre of every category, in plot coordinates.
    fn centres(&self, frame: Frame) -> Vec<f32> {
        let domain: Vec<usize> = (0..self.labels.len()).collect();
        if self.chart_type == ChartType::Bar {
            let scale = Self::band_scale(&domain, frame);
            let half = scale.band_width() / 2.0;
            return domain
                .iter()
                .map(|index| frame.left + scale.tick(index).unwrap_or_default() + half)
                .collect();
        }
        let scale = ScalePoint::new(domain.clone(), vec![0.0, frame.width]);
        domain
            .iter()
            .map(|index| frame.left + scale.tick(index).unwrap_or_default())
            .collect()
    }

    /// Indices are the band domain, not the labels: two rows may share a category name and
    /// each still deserves its own band.
    fn band_scale(domain: &[usize], frame: Frame) -> ScaleBand<usize> {
        ScaleBand::new(domain.to_vec(), vec![0.0, frame.width])
            .padding_inner(0.4)
            .padding_outer(0.2)
    }

    fn band_width(&self, frame: Frame) -> f32 {
        Self::band_scale(&(0..self.labels.len()).collect::<Vec<_>>(), frame).band_width()
    }

    fn paint_bars(&self, bounds: Bounds<Pixels>, frame: Frame, window: &mut Window, cx: &mut App) {
        let scale = self.value_scale(frame);
        let baseline = scale.tick(&0.0).unwrap_or(frame.bottom);
        let centres = self.centres(frame);
        let band = self.band_width(frame);
        let slot = band / count(self.series.len()).max(1.0);
        let width = (slot - self.scaled(BAR_GAP)).max(1.0);
        let radius = px(self.scaled(BAR_CORNER).min(width / 2.0));

        // Grouped bars walk outwards from the category centre, one slot at a time.
        let mut offset = -band / 2.0;
        for series in &self.series {
            let marks = series.marks(&centres, offset, &scale);
            offset += slot;
            let color = series.color;
            Bar::new()
                .data(marks)
                .alignment(BarAlignment::Bottom)
                .band_width(width)
                .cross(|mark: &Mark| Some(mark.x))
                .base(move |_| baseline)
                .value(|mark: &Mark| Some(mark.y))
                .fill(move |_, _, _| color)
                .corner_radii(Corners {
                    top_left: radius,
                    top_right: radius,
                    bottom_right: px(0.0),
                    bottom_left: px(0.0),
                })
                .paint(&bounds, window, cx);
        }
    }

    fn paint_curves(&self, bounds: Bounds<Pixels>, frame: Frame, window: &mut Window) {
        let scale = self.value_scale(frame);
        let baseline = scale.tick(&0.0).unwrap_or(frame.bottom);
        let centres = self.centres(frame);

        for series in &self.series {
            let marks = series.marks(&centres, 0.0, &scale);
            if self.chart_type == ChartType::Area {
                Area::new()
                    .data(marks.clone())
                    .x(|mark: &Mark| Some(mark.x))
                    .y0(baseline)
                    .y1(|mark: &Mark| Some(mark.y))
                    .fill(series.color.opacity(AREA_OPACITY))
                    // The outline is the `Line` below, at the full stroke width.
                    .stroke(series.color.opacity(0.0))
                    .stroke_style(StrokeStyle::Natural)
                    .paint(&bounds, window);
            }
            Line::new()
                .data(marks)
                .x(|mark: &Mark| Some(mark.x))
                .y(|mark: &Mark| Some(mark.y))
                .stroke(series.color)
                .stroke_width(px(self.scaled(LINE_WIDTH)))
                .stroke_style(StrokeStyle::Natural)
                .paint(&bounds, window);
        }
    }

    /// Category labels below the baseline, thinned until they stop colliding and each
    /// truncated to the room its own category has.
    fn paint_category_labels(
        &self,
        bounds: Bounds<Pixels>,
        frame: Frame,
        window: &mut Window,
        cx: &mut App,
    ) {
        let color = cx.peek_theme().fg_subtle;
        let (text_size, label_budget) = (px(self.scaled(TEXT_SIZE)), self.scaled(LABEL_BUDGET));
        let centres = self.centres(frame);
        let budget = centres
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(frame.width, f32::min);

        // Labels are dropped, never rotated, once a neighbour would collide. A running
        // cursor adapts to the real spacing instead of guessing a stride, and always keeps
        // the first category.
        let mut placed: Option<f32> = None;
        let texts = self
            .labels
            .iter()
            .zip(&centres)
            .filter(|(_, x)| match placed {
                Some(previous) if **x - previous < label_budget => false,
                _ => {
                    placed = Some(**x);
                    true
                }
            })
            .map(|(label, x)| {
                let label = truncate_text_to_width(label, text_size, budget, window);
                let top = frame.bottom + self.scaled(TEXT_GAP * 2.0);
                Text::new(label, point(px(*x), px(top)), color)
                    .font_size(text_size)
                    .align(TextAlign::Center)
            })
            .collect();
        PlotLabel::new(texts).paint(&bounds, window, cx);
    }

    fn paint_value_axis(
        &self,
        bounds: Bounds<Pixels>,
        frame: Frame,
        window: &mut Window,
        cx: &mut App,
    ) {
        let color = cx.peek_theme().fg_subtle;
        let scale = self.value_scale(frame);
        let (low, high) = self.extent();
        let text_size = self.scaled(TEXT_SIZE);
        let right = frame.left - self.scaled(TEXT_GAP);

        // Placed by hand rather than through `PlotAxis::y_label`, which offsets each label by
        // the unscaled text size and gap.
        let labels = (0..=VALUE_INTERVALS)
            .filter_map(|interval| {
                let value = low + (high - low) * f64::from(interval) / f64::from(VALUE_INTERVALS);
                let tick = scale.tick(&value)?;
                let origin = point(px(right), px(tick - text_size / 2.0));
                Some(
                    Text::new(format_value(value), origin, color)
                        .font_size(px(text_size))
                        .align(TextAlign::Right),
                )
            })
            .collect();
        PlotLabel::new(labels).paint(&bounds, window, cx);
    }

    /// The zero line, spanning the plot area only so it never runs under the value labels.
    fn paint_baseline(
        &self,
        bounds: Bounds<Pixels>,
        frame: Frame,
        window: &mut Window,
        cx: &mut App,
    ) {
        let scale = self.value_scale(frame);
        let baseline = scale.tick(&0.0).unwrap_or(frame.bottom);
        let plot_bounds = Bounds {
            origin: bounds.origin + point(px(frame.left), px(0.0)),
            size: Size::new(px(frame.width), bounds.size.height),
        };
        PlotAxis::new()
            .x(px(baseline))
            .stroke(cx.peek_theme().node_border)
            .paint(&plot_bounds, window, cx);
    }

    fn nearest(&self, x: f32, frame: Frame) -> Option<(usize, f32)> {
        self.centres(frame)
            .into_iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a - x)
                    .abs()
                    .partial_cmp(&(b - x).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

impl Plot for SeriesChart {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        if self.labels.is_empty() || self.series.is_empty() {
            return;
        }
        let frame = self.frame(bounds);
        self.paint_value_axis(bounds, frame, window, cx);
        self.paint_baseline(bounds, frame, window, cx);
        if self.chart_type == ChartType::Bar {
            self.paint_bars(bounds, frame, window, cx);
        } else {
            self.paint_curves(bounds, frame, window);
        }
        self.paint_category_labels(bounds, frame, window, cx);
    }

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &App,
    ) -> Option<TooltipState> {
        let frame = self.frame(bounds);
        let (index, x) = self.nearest(position.x.as_f32(), frame)?;
        // The markers are built in `tooltip`, where each can carry its own series' colour.
        Some(TooltipState::new(index, point(px(x), px(0.0)), Vec::new()))
    }

    fn tooltip(
        &self,
        state: &TooltipState,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let frame = self.frame(bounds);
        let label = self.labels.get(state.index)?;
        let surface = cx.peek_theme().node_bg;
        let scale = self.value_scale(frame);

        // Rebuilt from the series rather than read off `state.dots`, so each marker keeps
        // its own colour; the surface ring is what separates overlapping ones.
        let hovered = self.series.iter().filter_map(|series| {
            let value = (*series.values.get(state.index)?)?;
            let dot = point(state.cross_line.x, px(scale.tick(&value)?));
            Some((series, value, dot))
        });

        let mut dots = Vec::new();
        let mut tooltip = Tooltip::new(cursor, bounds.size)
            .title(label.clone())
            .cross_line(
                CrossLine::new(state.cross_line)
                    .band(self.band_width(frame).max(self.scaled(LINE_WIDTH)))
                    .span(frame.top, frame.bottom - frame.top),
            );
        for (series, value, dot) in hovered {
            dots.push(
                Dot::new(dot)
                    .size(px(self.scaled(DOT_SIZE)))
                    .fill(series.color)
                    .stroke(surface),
            );
            tooltip = tooltip.row(series.color, series.name.clone(), format_value(value));
        }
        Some(tooltip.dots(dots).into_any_element())
    }
}

impl Series {
    /// This series' points, already in plot coordinates. `offset` shifts grouped bars off
    /// their category's centre; curves pass zero.
    fn marks(&self, centres: &[f32], offset: f32, scale: &ScaleLinear<f64>) -> Vec<Mark> {
        self.values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                Some(Mark {
                    x: centres.get(index)? + offset,
                    y: scale.tick(&(*value)?)?,
                })
            })
            .collect()
    }
}

/// The plot area inside the element's bounds, in pixels from its origin.
#[derive(Debug, Clone, Copy)]
struct Frame {
    left: f32,
    top: f32,
    bottom: f32,
    width: f32,
}

#[derive(Debug, Clone, Copy)]
struct Mark {
    x: f32,
    y: f32,
}

/// Counts as a float, without a lossy cast: series and categories never come near
/// `u16::MAX`, and a chart with more categories than that has nothing legible to lay out.
fn count(value: usize) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

/// Axis and tooltip numbers, short enough for a 34px gutter: `1.2k`, `4.5M`, `17`.
fn format_value(value: f64) -> String {
    for (limit, suffix) in [(1e9, "B"), (1e6, "M"), (1e3, "k")] {
        if value.abs() >= limit {
            return format!("{}{suffix}", trim(value / limit));
        }
    }
    trim(value)
}

fn trim(value: f64) -> String {
    let text = format!("{value:.2}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_formatted_short_enough_for_the_gutter() {
        assert_eq!(format_value(0.0), "0");
        assert_eq!(format_value(17.0), "17");
        assert_eq!(format_value(0.25), "0.25");
        assert_eq!(format_value(1_200.0), "1.2k");
        assert_eq!(format_value(4_500_000.0), "4.5M");
        assert_eq!(format_value(-2_000_000_000.0), "-2B");
    }
}
