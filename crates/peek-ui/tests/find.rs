//! Page-wide node search and the result pivot: `Page::Search` and `Result::Pivot`.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_document::geometry::Size;
use peek_document::{CanvasDocument, Cell, Column, NodeId, ResultData, ResultSet};
use peek_ui::WorkspaceView;

/// A query, its result, and two text nodes far enough apart that framing one is unmistakable.
const DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [
        {
          "id": "text_aaaaaaaa",
          "type": "text",
          "position": { "x": 0, "y": 0 },
          "width": 300,
          "height": 200,
          "data": { "text": "grocery list" }
        },
        {
          "id": "text_bbbbbbbb",
          "type": "text",
          "position": { "x": 4000, "y": 4000 },
          "width": 300,
          "height": 200,
          "data": { "text": "deployment runbook" }
        },
        {
          "id": "query_cccccccc",
          "type": "query",
          "position": { "x": 0, "y": 800 },
          "width": 350,
          "height": 240,
          "data": { "query": "select * from invoices" }
        },
        {
          "id": "query_cccccccc-result-0",
          "type": "result",
          "position": { "x": 500, "y": 800 },
          "width": 620,
          "height": 640,
          "data": { "query": "select * from invoices" }
        }
      ],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

fn result_node() -> NodeId {
    NodeId::from("query_cccccccc-result-0")
}

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let document = CanvasDocument::from_json(DOCUMENT).unwrap();
        let view = cx.new(|cx| {
            let view = WorkspaceView::with_document("test", document, window, cx);
            view.document(cx).update(cx, |document, _| {
                document.set_result(
                    result_node(),
                    ResultSet::new(
                        vec![Column::new("id", "INT4"), Column::new("customer", "TEXT")],
                        vec![
                            vec![Cell::Int(1), Cell::Text("northwind".to_string())],
                            vec![Cell::Int(2), Cell::Text("umbrella".to_string())],
                        ],
                    ),
                );
            });
            view
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, workspace.unwrap())
}

fn selection(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<String> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .document(cx)
            .read(cx)
            .selected()
            .iter()
            .map(ToString::to_string)
            .collect()
    })
}

fn result_data(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> ResultData {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let node = document.node(&result_node()).expect("the result is there");
        peek_document::NodeData::get(&node.kind)
            .cloned()
            .expect("it is a result")
    })
}

fn node_size(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Size {
    cx.update(|cx| {
        workspace
            .read(cx)
            .document(cx)
            .read(cx)
            .node(&result_node())
            .expect("the result is there")
            .size()
    })
}

fn open_page_search(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("cmd-f", cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn type_query(cx: &mut TestAppContext, handle: WindowHandle<Root>, query: &str) {
    cx.update_window(handle.into(), |_, window, cx| window.input(query, cx))
        .unwrap();
    // `on_query` is deferred by `CommandState`, and so is the fly it triggers.
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// `cmd-f` with nothing focused is the page-wide search, not the result node's find bar.
#[gpui_kit::test]
fn cmd_f_opens_the_page_search(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    let before = cx
        .update_window(handle.into(), |_, window, _| {
            window.try_find("page-search").is_some()
        })
        .unwrap();
    assert!(!before, "nothing is open before the key");

    open_page_search(cx, handle);

    let after = cx
        .update_window(handle.into(), |_, window, _| {
            window.try_find("page-search").is_some()
        })
        .unwrap();
    assert!(after, "the panel opened");
}

/// The point of the panel: typing selects the best-matching node, which is what the camera
/// then follows. A node the query does not name must not win.
#[gpui_kit::test]
fn typing_selects_the_node_whose_contents_match(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    open_page_search(cx, handle);
    type_query(cx, handle, "runbook");

    assert_eq!(selection(cx, &workspace), ["text_bbbbbbbb"]);
}

/// A result node matches on the cells it holds, never on the SQL above it — that belongs to the
/// query node, and matching both would list one statement twice.
#[gpui_kit::test]
fn a_result_matches_on_its_rows(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    open_page_search(cx, handle);
    type_query(cx, handle, "umbrella");

    assert_eq!(selection(cx, &workspace), [result_node().to_string()]);
}

#[gpui_kit::test]
fn escape_closes_the_page_search(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    open_page_search(cx, handle);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();

    let open = cx
        .update_window(handle.into(), |_, window, _| {
            window.try_find("page-search").is_some()
        })
        .unwrap();
    assert!(!open, "the panel closed");
}

/// `Result::Pivot` acts on the selection, so it has to fire from the palette's dispatch path —
/// the canvas focus handle — and not only from the node.
#[gpui_kit::test]
fn pivot_transposes_the_selected_result_and_remembers_its_size(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update(|cx| {
        workspace
            .read(cx)
            .document(cx)
            .update(cx, |document, _| document.select_only([result_node()]));
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("shift-p", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let data = result_data(cx, &workspace);
    assert_eq!(data.pivoted, Some(true));
    assert_eq!(data.pre_pivot_size, Some(Size::new(620.0, 640.0)));
}

/// One press is one undo step: the reshape and the flag have to come back together.
#[gpui_kit::test]
fn pivoting_twice_restores_the_node(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update(|cx| {
        workspace
            .read(cx)
            .document(cx)
            .update(cx, |document, _| document.select_only([result_node()]));
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("shift-p", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // 460 wide, and `130 + 2 columns * 34` clamped up to the 260 floor — `togglePivot.ts`.
    assert_eq!(
        node_size(cx, &workspace),
        Size::new(460.0, 260.0),
        "the record view reshapes the node"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("shift-p", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let data = result_data(cx, &workspace);
    assert_eq!(data.pivoted, None);
    assert_eq!(data.pre_pivot_size, None);
    assert_eq!(node_size(cx, &workspace), Size::new(620.0, 640.0));
}
