//! The result node's sub-toolbar: `ResultToolbar.tsx`.
//!
//! Four exclusive states, the first three in the reference's own precedence: the search bar
//! while a find is open, the format strip while Export or Copy is asking which one, the
//! selection statistics while a numeric rectangle is selected, and otherwise the meta row — a
//! status dot, the row count, and a badge per table the query reads.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable, Icon, Selectable, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, AnyElement, App, Context, Entity, Hsla, SharedString, Window, div, px, rems,
};
use peek_theme::ActivePeekTheme;

use super::ResultTable;
use super::aggregate::{Aggregates, format_number};
use super::menu::actions::Format;
use crate::commands::actions;

/// `.icon-btn`: a square button carrying a 14 px glyph, in a 26 px frame at zoom 1.
const BUTTON: f32 = 1.625;
const GLYPH: f32 = 0.875;

/// `.table-badge` and `.col-tag`: a small corner, well below the theme's card radius, so a chip
/// inside a table row does not read as a card.
const BADGE_RADIUS: f32 = 3.0;
/// The alphas `.stat-chip` mixes its pill out of `--chip-color`.
const CHIP_FILL: f32 = 0.11;
const CHIP_BORDER: f32 = 0.28;

/// Where the rows are going, once the user has picked a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Destination {
    /// A file, in a directory the user picks.
    Export,
    /// The clipboard.
    Copy,
}

impl Destination {
    const fn verb(self) -> &'static str {
        match self {
            Self::Export => "Export as",
            Self::Copy => "Copy as",
        }
    }

    /// The command behind one format. Both destinations go through the node's own scoped
    /// commands, so a toolbar export with a band of rows selected exports that band — the same
    /// rule the right-click menu follows, and the reason there is no separate whole-result path.
    fn action(self, format: Format) -> Box<dyn Action> {
        match (self, format) {
            (Self::Export, Format::Json) => Box::new(actions::result::ExportAsJson),
            (Self::Export, Format::Csv) => Box::new(actions::result::ExportAsCsv),
            (Self::Export, Format::Sql) => Box::new(actions::result::ExportAsSql),
            (Self::Copy, Format::Json) => Box::new(actions::result::CopyAsJson),
            (Self::Copy, Format::Csv) => Box::new(actions::result::CopyAsCsv),
            (Self::Copy, Format::Sql) => Box::new(actions::result::CopyAsSql),
        }
    }
}

impl ResultTable {
    pub(super) fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme();
        let row = div()
            .h_flex()
            .items_center()
            .justify_between()
            .flex_none()
            .px(rems(0.75))
            .py(rems(0.5))
            .gap(rems(0.75))
            .border_b_1()
            .border_color(theme.node_border);

        if self.search_open {
            return row.child(self.search_bar(cx)).into_any_element();
        }
        if let Some(destination) = self.format_menu {
            return row
                .child(self.format_strip(destination, cx))
                .into_any_element();
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
            .gap(rems(0.75))
            .min_w_0()
            .text_size(rems(0.71875))
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
                    .px(rems(0.375))
                    .py(rems(0.0625))
                    .rounded(px(BADGE_RADIUS))
                    .border_1()
                    .border_color(theme.node_border)
                    .bg(theme.node_bg_2)
                    .text_color(theme.fg)
                    .child(name.clone())
                    .into_any_element()
            })
            .collect()
    }

    /// The right-hand buttons: the reference's icon row, minus the three whose features have not
    /// landed (add row, ask, fork).
    fn actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let has_rows = self.table.read(cx).delegate().result_rows().row_count() > 0;
        let pivoted = self.table.read(cx).delegate().is_pivoted();
        div()
            .h_flex()
            .items_center()
            .gap(rems(0.125))
            .flex_none()
            .children(self.deletable_rows(cx).map(|count| {
                // Only when rows are selected in a result that can be written, so the one
                // irreversible action in the table never sits there inviting a stray click.
                Button::new(self.ids.delete.clone())
                    .danger()
                    .xsmall()
                    .label(if count == 1 {
                        "Delete row".to_string()
                    } else {
                        format!("Delete {count} rows")
                    })
                    .tooltip("Delete the selected rows — this cannot be undone")
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_delete(window, cx)))
            }))
            .child(
                icon_button(self.ids.chart.clone(), IconName::ChartColumn)
                    // A chart of a column of names is a chart of nothing, so the button waits
                    // for a column it can actually plot.
                    .disabled(!self.chartable(cx))
                    .tooltip("Create chart")
                    .on_click(cx.listener(|this, _, window, cx| this.dispatch_chart(window, cx))),
            )
            .child(
                icon_button(self.ids.export.clone(), IconName::Download)
                    .disabled(!has_rows)
                    .tooltip("Export")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_format_menu(Destination::Export, cx);
                    })),
            )
            .child(
                icon_button(self.ids.copy.clone(), IconName::Copy)
                    .disabled(!has_rows)
                    .tooltip("Copy")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_format_menu(Destination::Copy, cx);
                    })),
            )
            .child(
                icon_button(self.ids.pivot.clone(), IconName::Rows3)
                    .selected(pivoted)
                    .disabled(!has_rows)
                    .tooltip(if pivoted {
                        "Show as a table"
                    } else {
                        "Read the rows as records"
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.dispatch_pivot(window, cx))),
            )
            .child(
                icon_button(self.ids.search.clone(), IconName::Search)
                    // The record view has no rows to filter, so the find bar has nothing to do
                    // there — the same exclusion the reference draws.
                    .disabled(!has_rows || pivoted)
                    .tooltip("Search results")
                    .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
            )
            .into_any_element()
    }

    /// Export and Copy each want a format. The reference opens a dropdown; an overlay raised
    /// from inside a node body lays out at rem 1 whatever the camera is doing, and inherits the
    /// body's `overflow_hidden` mask, so this takes over the toolbar row instead — the same
    /// reversal the value pane made.
    fn format_strip(&self, destination: Destination, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme();
        div()
            .h_flex()
            .items_center()
            .gap(rems(0.375))
            .w_full()
            .text_size(rems(0.6875))
            .text_color(theme.fg_muted)
            .child(div().flex_none().child(destination.verb()))
            .children(
                [Format::Json, Format::Csv, Format::Sql]
                    .map(|format| self.format_button(format, destination, cx)),
            )
            .child(div().flex_1())
            .child(
                Button::new(self.ids.format_close.clone())
                    .ghost()
                    .xsmall()
                    .label("Close")
                    .on_click(cx.listener(|this, _, _, cx| this.close_format_menu(cx))),
            )
            .into_any_element()
    }

    /// One format. It selects its own node first, because the commands read the canvas selection
    /// rather than a focused node — the reason `dispatch_pivot` does the same.
    fn format_button(
        &self,
        format: Format,
        destination: Destination,
        cx: &mut Context<Self>,
    ) -> Button {
        let label = format.label();
        let id = SharedString::from(format!("{}-format-{label}", self.node));
        let action = destination.action(format);
        Button::new(id)
            .ghost()
            .xsmall()
            .label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.close_format_menu(cx);
                this.select_self(cx);
                window.dispatch_action(action.boxed_clone(), cx);
            }))
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
                    .flex_none()
                    .child(Icon::new(IconName::Search).size(rems(GLYPH)))
                    .text_color(theme.fg_subtle),
            )
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
                Button::new(self.ids.search_close.clone())
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

/// count / sum / avg / min / max as `.stat-chip` pills, in tabular figures so the numbers line
/// up as they change.
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
        .gap(rems(0.375))
        .min_w_0()
        .overflow_hidden()
        .text_size(rems(0.6875))
        .children(chips.into_iter().map(|(label, value)| {
            chip(theme.fg_muted)
                .child(
                    div()
                        .text_size(rems(0.625))
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

/// `.stat-chip`: a pill mixed out of one colour, so a label and its value read as one token.
fn chip(color: Hsla) -> gpui_kit::Div {
    div()
        .h_flex()
        .items_center()
        .gap(rems(0.25))
        .flex_none()
        .px(rems(0.5))
        .py(rems(0.125))
        .rounded_full()
        .border_1()
        .border_color(color.opacity(CHIP_BORDER))
        .bg(color.opacity(CHIP_FILL))
}

/// `.icon-btn`. The glyph is a child rather than `Button::icon`, which would size it to three
/// quarters of the frame — the reason `canvas/toolbar.rs` builds its buttons the same way.
fn icon_button(id: SharedString, icon: IconName) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .size(rems(BUTTON))
        .p_0()
        .child(Icon::new(icon).size(rems(GLYPH)))
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
    }
}

/// The search field, created once per result node.
pub(super) fn search_input(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder("Search results…"))
}
