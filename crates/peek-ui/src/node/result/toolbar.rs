//! The result node's sub-toolbar: `ResultToolbar.tsx`.
//!
//! Three exclusive states, in the reference's own precedence: the search bar while a find is
//! open, the selection statistics while a numeric rectangle is selected, and otherwise the meta
//! row — a status dot, the row count, and a badge per table the query reads.
//!
//! Buttons whose feature has not landed render disabled with a tooltip naming what they wait
//! for, which is the convention `canvas/toolbar.rs` already uses for unregistered tools.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, SharedString, div, rems};
use peek_theme::ActivePeekTheme;

use super::ResultTable;
use super::aggregate::{Aggregates, format_number};

impl ResultTable {
    pub(super) fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme();
        let row = div()
            .h_flex()
            .items_center()
            .justify_between()
            .flex_none()
            .h(rems(1.75))
            .px(rems(0.5))
            .gap(rems(0.5))
            .border_b_1()
            .border_color(theme.node_border);

        if self.search_open {
            return row.child(self.search_bar(cx)).into_any_element();
        }
        row.child(self.meta(cx))
            .child(self.actions(cx))
            .into_any_element()
    }

    /// The left-hand side: statistics when a numeric selection is live, the row count otherwise.
    fn meta(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(stats) = self.selection_stats(cx) {
            return statistics(&stats, cx);
        }
        let theme = cx.peek_theme();
        let table = self.table.read(cx);
        let delegate = table.delegate();
        let total = delegate.result_rows().row_count();

        div()
            .h_flex()
            .items_center()
            .gap(rems(0.375))
            .min_w_0()
            .text_size(rems(0.6875))
            .text_color(theme.fg_muted)
            .child(
                div()
                    .size(rems(0.375))
                    .rounded_full()
                    .flex_none()
                    .bg(theme.green),
            )
            .child(plural(total, "row", "rows"))
            .children(self.table_badges(cx))
            .into_any_element()
    }

    /// One badge per table the query reads, from `peek_lsp`'s statement analysis.
    fn table_badges(&self, cx: &App) -> Vec<AnyElement> {
        let theme = cx.peek_theme();
        self.tables
            .iter()
            .map(|name| {
                div()
                    .flex_none()
                    .px(rems(0.3125))
                    .py(rems(0.0625))
                    .rounded(theme.radius_pill)
                    .bg(theme.node_bg_2)
                    .text_color(theme.fg_subtle)
                    .child(name.clone())
                    .into_any_element()
            })
            .collect()
    }

    /// The right-hand buttons. Only search works today; the rest name what they wait for.
    fn actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let has_rows = self.table.read(cx).delegate().result_rows().row_count() > 0;
        div()
            .h_flex()
            .items_center()
            .gap(rems(0.125))
            .flex_none()
            .child(
                Button::new(SharedString::from(format!("{}-search", self.node)))
                    .ghost()
                    .xsmall()
                    .label("Find")
                    .disabled(!has_rows)
                    .tooltip("Search this result")
                    .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
            )
            .child(waiting(
                "Chart",
                "Charting a result arrives with the chart tools",
            ))
            .child(waiting(
                "Export",
                "Export arrives with the export milestone",
            ))
            .child(waiting(
                "Pivot",
                "The record view arrives with the pivot milestone",
            ))
            .into_any_element()
    }

    fn search_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme();
        let table = self.table.read(cx);
        let shown = table.delegate().matches().len();
        let searching = !self.search_query.trim().is_empty();

        div()
            .h_flex()
            .items_center()
            .gap(rems(0.375))
            .w_full()
            .text_size(rems(0.6875))
            .text_color(theme.fg_muted)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.search_input).xsmall()),
            )
            .when(searching, |row| {
                row.child(div().flex_none().child(plural(shown, "match", "matches")))
            })
            .child(
                Button::new(SharedString::from(format!("{}-search-close", self.node)))
                    .ghost()
                    .xsmall()
                    .label("Close")
                    .tooltip("Close find")
                    .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))),
            )
            .into_any_element()
    }

    fn selection_stats(&self, cx: &App) -> Option<Aggregates> {
        let table = self.table.read(cx);
        let delegate = table.delegate();
        let rect = delegate.selection().rect()?;
        super::aggregate::aggregate(delegate.result_rows(), rect, delegate.matches().visible())
    }
}

/// count / sum / avg / min / max, in tabular figures so the numbers line up as they change.
fn statistics(stats: &Aggregates, cx: &App) -> AnyElement {
    let theme = cx.peek_theme();
    let chips = [
        ("COUNT", format_number(stats.count)),
        ("SUM", format_number(stats.sum)),
        ("AVG", format_number(stats.average)),
        ("MIN", format_number(stats.minimum)),
        ("MAX", format_number(stats.maximum)),
    ];
    div()
        .h_flex()
        .items_center()
        .gap(rems(0.625))
        .min_w_0()
        .overflow_hidden()
        .text_size(rems(0.6875))
        .children(chips.into_iter().map(|(label, value)| {
            div()
                .h_flex()
                .items_center()
                .gap(rems(0.25))
                .flex_none()
                .child(
                    div()
                        .text_size(rems(0.5625))
                        .text_color(theme.fg_subtle)
                        .child(label),
                )
                .child(
                    div()
                        .font_family("Monaspace Krypton")
                        .text_color(theme.fg)
                        .child(value),
                )
        }))
        .into_any_element()
}

/// A button for a feature that has not landed, which says so rather than doing nothing. The
/// convention `canvas/toolbar.rs` already uses for tools with no command behind them yet.
fn waiting(label: &'static str, reason: &'static str) -> AnyElement {
    Button::new(SharedString::from(format!("result-waiting-{label}")))
        .ghost()
        .xsmall()
        .label(label)
        .disabled(true)
        .tooltip(reason)
        .into_any_element()
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
    }
}

/// The search field, created once per result node.
pub(super) fn search_input(window: &mut gpui_kit::Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder("Search results…"))
}
