//! Rerunning the page's queries, and exporting a selected result's rows.
//!
//! All four commands read the selection (or the page) rather than a focused node, so they are
//! handled here — see the module docs in `dispatch/mod.rs` for why a node-only handler would
//! never be reached from the palette.

use std::path::Path;

use gpui_kit::{Context, InteractiveElement, PathPromptOptions, Window};
use peek_canvas::Document;
use peek_document::{Node, NodeData, NodeId, NodeType, QueryData, ResultData, ResultSet};

use super::CanvasView;
use crate::commands::actions;
use crate::node::state::NodeState;

/// `rerunAllQueriesOnPage.tsx` waits 20 ms between runs so a page of queries does not open
/// every connection in the pool at once.
const STAGGER: std::time::Duration = std::time::Duration::from_millis(20);

pub(super) fn register<E: InteractiveElement>(element: E, cx: &mut Context<CanvasView>) -> E {
    element
        .on_action(cx.listener(CanvasView::rerun_all_queries))
        .on_action(cx.listener(CanvasView::rerun_selected_queries))
        .on_action(cx.listener(CanvasView::export_selected_csv))
        .on_action(cx.listener(CanvasView::export_selected_json))
}

/// Which serialiser an export command wants, and the extension the file gets.
#[derive(Debug, Clone, Copy)]
enum Format {
    Csv,
    Json,
}

impl Format {
    const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }

    fn serialize(self, rows: &ResultSet) -> String {
        match self {
            Self::Csv => peek_document::to_csv(rows),
            Self::Json => peek_document::to_json(rows),
        }
    }
}

impl CanvasView {
    fn rerun_all_queries(
        &mut self,
        _: &actions::query::RerunAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        rerun(queries_to_rerun(self.document.read(cx), false), window, cx);
    }

    fn rerun_selected_queries(
        &mut self,
        _: &actions::query::RerunSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        rerun(queries_to_rerun(self.document.read(cx), true), window, cx);
    }

    /// One run, through the node's own state so the editor's confirmation flag and running
    /// indicator stay in step with a run started from the palette.
    fn run_query(&mut self, node: &NodeId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.document.read(cx).node(node).cloned() else {
            return;
        };
        if let Some(NodeState::Query(state)) = self.node_states.get(&node, window, cx) {
            state.run(cx);
        }
    }

    fn export_selected_csv(
        &mut self,
        _: &actions::export::Csv,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.export_selected(Format::Csv, cx);
    }

    fn export_selected_json(
        &mut self,
        _: &actions::export::Json,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.export_selected(Format::Json, cx);
    }

    /// Serialises every selected result up front, then asks for a directory.
    ///
    /// The reference joins the picked absolute path onto `BaseDirectory.AppConfig`, so its
    /// exports land next to the config rather than where they were asked for. This writes the
    /// absolute path.
    fn export_selected(&mut self, format: Format, cx: &mut Context<Self>) {
        let files = self.selected_result_files(format, cx);
        if files.is_empty() {
            return;
        }

        let prompt = cx.prompt_for_paths(PathPromptOptions {
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
                .spawn(async move { write_all(&directory, files) })
                .await;
        })
        .detach();
    }

    /// `(filename, contents)` for each selected result node, in document order.
    ///
    /// Two results of the same SQL slugify to the same name and the second overwrites the
    /// first, exactly as in the reference.
    fn selected_result_files(&self, format: Format, cx: &Context<Self>) -> Vec<(String, String)> {
        let document = self.document.read(cx);
        self.selected_of_kind(NodeType::Result, cx)
            .iter()
            .filter_map(|id| {
                let node = document.node(id)?;
                let sql = ResultData::get(&node.kind).map(|data| data.query.as_str())?;
                let rows = document.result(id).cloned().unwrap_or_default();
                Some((
                    peek_document::export_filename(sql, format.extension()),
                    format.serialize(&rows),
                ))
            })
            .collect()
    }
}

/// Starts each run 20 ms after the last. Detached rather than retained: the loop ends by itself,
/// and `update_in` fails the moment the canvas goes away.
fn rerun(queries: Vec<NodeId>, window: &mut Window, cx: &mut Context<CanvasView>) {
    if queries.is_empty() {
        return;
    }
    cx.spawn_in(window, async move |this, cx| {
        for (index, node) in queries.into_iter().enumerate() {
            if index > 0 {
                cx.background_executor().timer(STAGGER).await;
            }
            if this
                .update_in(cx, |view, window, cx| view.run_query(&node, window, cx))
                .is_err()
            {
                return;
            }
        }
    })
    .detach();
}

/// The page's query nodes in run order: left to right, blank SQL dropped.
///
/// The reference runs them in React Flow's own node order, which is insertion order and so has
/// nothing to do with how the page reads. Sorting by x makes a page of queries rerun in the
/// order the eye follows, and makes the order reproducible.
fn queries_to_rerun(document: &Document, only_selected: bool) -> Vec<NodeId> {
    let mut queries: Vec<&Node> = document
        .nodes()
        .iter()
        .filter(|node| node.node_type() == Some(NodeType::Query))
        .filter(|node| !only_selected || document.selected().contains(&node.id))
        .filter(|node| QueryData::get(&node.kind).is_some_and(|data| !data.query.trim().is_empty()))
        .collect();
    queries.sort_by(|left, right| left.position.x.total_cmp(&right.position.x));
    queries.iter().map(|node| node.id.clone()).collect()
}

/// A failed write is logged per file rather than aborting: one unwritable name must not cost
/// the user the other exports they asked for.
fn write_all(directory: &Path, files: Vec<(String, String)>) {
    for (name, contents) in files {
        let path = directory.join(&name);
        if let Err(error) = std::fs::write(&path, contents) {
            log::error!("peek: could not export {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use peek_canvas::Document;
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{CanvasDocument, NodeId, NodeType, QueryData};

    use super::queries_to_rerun;

    /// A query node at `x`, holding `sql`.
    fn query(document: &mut Document, x: f64, sql: &str) -> NodeId {
        let id = document.create_node(
            NodeType::Query,
            Rect::new(Point::new(x, 0.0), Size::new(400.0, 300.0)),
        );
        document.update_data::<QueryData>(&id, |data| data.query = sql.to_string());
        id
    }

    #[test]
    fn the_page_runs_left_to_right_whatever_order_it_was_built_in() {
        let mut document = Document::load(CanvasDocument::empty());
        let right = query(&mut document, 900.0, "select 3");
        let left = query(&mut document, -200.0, "select 1");
        let middle = query(&mut document, 400.0, "select 2");

        assert_eq!(
            queries_to_rerun(&document, false),
            vec![left, middle, right],
            "insertion order is not reading order"
        );
    }

    /// `q.trim()` in the reference: a node left blank must not be sent to the database as an
    /// empty statement.
    #[test]
    fn a_blank_query_is_skipped() {
        let mut document = Document::load(CanvasDocument::empty());
        query(&mut document, 0.0, "   \n\t ");
        let real = query(&mut document, 100.0, "select 1");

        assert_eq!(queries_to_rerun(&document, false), vec![real]);
    }

    #[test]
    fn nothing_but_query_nodes_is_rerun() {
        let mut document = Document::load(CanvasDocument::empty());
        document.create_node(
            NodeType::Result,
            Rect::new(Point::default(), Size::new(600.0, 440.0)),
        );
        let real = query(&mut document, 0.0, "select 1");

        assert_eq!(queries_to_rerun(&document, false), vec![real]);
    }

    #[test]
    fn the_selected_form_is_the_same_loop_narrowed_to_the_selection() {
        let mut document = Document::load(CanvasDocument::empty());
        let first = query(&mut document, 0.0, "select 1");
        let second = query(&mut document, 500.0, "select 2");
        document.select_only([second.clone()]);

        assert_eq!(queries_to_rerun(&document, true), vec![second]);
        assert_eq!(queries_to_rerun(&document, false).len(), 2, "{first} too");
    }
}
