use gpui_kit::{Context, Entity, InteractiveElement, Window};
use peek_canvas::flight::durations;
use peek_document::NodeType;

use super::CanvasView;
use crate::commands::actions;
use crate::node::result::menu::actions::Format;

pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    element
        .on_action(cx.listener(CanvasView::pivot_results))
        .on_action(cx.listener(CanvasView::chart_results))
        .on_action(cx.listener(CanvasView::copy_result_value))
        .on_action(cx.listener(CanvasView::copy_result_json))
        .on_action(cx.listener(CanvasView::copy_result_csv))
        .on_action(cx.listener(CanvasView::copy_result_sql))
        .on_action(cx.listener(CanvasView::export_result_json))
        .on_action(cx.listener(CanvasView::export_result_csv))
        .on_action(cx.listener(CanvasView::export_result_sql))
        .on_action(cx.listener(CanvasView::use_result_as_variable))
        .on_action(cx.listener(CanvasView::delete_result_rows))
}

impl CanvasView {
    /// `Result::Pivot` flips every selected result between the table and the record view.
    fn pivot_results(
        &mut self,
        _: &actions::result::Pivot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.selected_of_kind(NodeType::Result, cx);
        let Some(only) = ids.first().filter(|_| ids.len() == 1).cloned() else {
            self.toggle_pivot(&ids, cx);
            return;
        };
        self.toggle_pivot(&ids, cx);
        // A pivoted node changes shape, so on its own it usually re-lays-out off screen. With
        // several selected the camera cannot follow them all, and the reference skips it too.
        self.frame_node(&only, durations::FIT_SELECTED, window, cx);
    }

    /// `Result::Chart` plots every selected result, and frames the one chart it just created.
    ///
    /// `createChart.ts` selects the chart and flies to it. With several results selected there is
    /// no single node to fly to, so the camera stays where it is — the same call the reference's
    /// pivot makes, and for the same reason.
    fn chart_results(
        &mut self,
        _: &actions::result::Chart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.selected_of_kind(NodeType::Result, cx);
        let charted: Vec<_> = self.document.update(cx, |document, cx| {
            let charted: Vec<_> = ids
                .iter()
                .filter_map(|id| document.place_chart(id))
                .collect();
            if !charted.is_empty() {
                cx.notify();
            }
            charted
        });
        let Some((chart, _)) = charted
            .iter()
            .find(|(_, created)| *created)
            .filter(|_| charted.len() == 1)
        else {
            return;
        };
        let chart = chart.clone();
        self.document.update(cx, |document, cx| {
            document.select_only([chart.clone()]);
            cx.notify();
        });
        self.frame_node(&chart, durations::FIT_SELECTED, window, cx);
    }

    /// The result table the scoped commands act on: the one selected result node.
    ///
    /// Several selected results have no single scope between them — a rectangle belongs to one
    /// table — so the commands act on exactly one and do nothing otherwise, where `Export::Csv`
    /// deliberately fans out over all of them.
    fn scoped_result(
        &self,
        cx: &Context<Self>,
    ) -> Option<Entity<crate::node::result::ResultTable>> {
        let ids = self.selected_of_kind(NodeType::Result, cx);
        let only = ids.first().filter(|_| ids.len() == 1)?;
        self.result_inner(only)
    }

    fn copy_result_value(
        &mut self,
        _: &actions::result::CopyValue,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(table) = self.scoped_result(cx) {
            table.update(cx, |table, cx| table.copy_value(cx));
        }
    }

    fn copy_result_json(
        &mut self,
        _: &actions::result::CopyAsJson,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_result(Format::Json, cx);
    }

    fn copy_result_csv(
        &mut self,
        _: &actions::result::CopyAsCsv,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_result(Format::Csv, cx);
    }

    fn copy_result_sql(
        &mut self,
        _: &actions::result::CopyAsSql,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_result(Format::Sql, cx);
    }

    fn copy_result(&mut self, format: Format, cx: &mut Context<Self>) {
        if let Some(table) = self.scoped_result(cx) {
            table.update(cx, |table, cx| table.copy_as(format, cx));
        }
    }

    fn export_result_json(
        &mut self,
        _: &actions::result::ExportAsJson,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.export_result(Format::Json, cx);
    }

    fn export_result_csv(
        &mut self,
        _: &actions::result::ExportAsCsv,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.export_result(Format::Csv, cx);
    }

    fn export_result_sql(
        &mut self,
        _: &actions::result::ExportAsSql,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.export_result(Format::Sql, cx);
    }

    /// Serialises first, then asks where to put it — so a cancelled dialog costs nothing and the
    /// rows cannot change under the prompt.
    fn export_result(&mut self, format: Format, cx: &mut Context<Self>) {
        let Some(table) = self.scoped_result(cx) else {
            return;
        };
        let Some((name, contents)) = table.read(cx).export_payload(format, cx) else {
            return;
        };
        let prompt = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Export".into()),
        });
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(directory))) = prompt.await else {
                return;
            };
            let Some(directory) = directory.into_iter().next() else {
                return;
            };
            cx.background_executor()
                .spawn(async move {
                    let path = directory.join(&name);
                    if let Err(error) = std::fs::write(&path, contents) {
                        log::error!("peek: could not export {}: {error}", path.display());
                    }
                })
                .await;
        })
        .detach();
    }

    fn use_result_as_variable(
        &mut self,
        _: &actions::result::UseAsVariable,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(table) = self.scoped_result(cx) {
            table.update(cx, crate::node::result::ResultTable::use_as_variable);
        }
    }

    fn delete_result_rows(
        &mut self,
        _: &actions::result::DeleteRows,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(table) = self.scoped_result(cx) {
            table.update(cx, |table, cx| table.delete_rows(window, cx));
        }
    }

    fn toggle_pivot(&mut self, ids: &[peek_document::NodeId], cx: &mut Context<Self>) {
        self.document.update(cx, |document, cx| {
            let changed = ids
                .iter()
                .filter(|id| crate::node::result::pivot::toggle(document, id))
                .count();
            if changed > 0 {
                cx.notify();
            }
        });
    }
}
