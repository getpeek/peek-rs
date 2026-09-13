//! The edge layer: a press on a curve selects the edge, Delete removes it, undo brings it back.
//! Driven through real gpui event dispatch.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Entity, Pixels, Point, TestAppContext, WindowHandle, point, px, size,
};
use peek_document::{CanvasDocument, EdgeId};
use peek_ui::WorkspaceView;

/// Two 200x200 text nodes on the same row, wired left to right. Their centres share `y = 200`,
/// so the floating bezier degenerates to the straight line `y = 200` from `x = 300` to
/// `x = 600` — a stretch of empty canvas whose midpoint is (450, 200) in world units.
const WIRED: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [
        { "id": "text_source00", "type": "text", "position": { "x": 100, "y": 100 },
          "width": 200, "height": 200, "data": { "text": "a" } },
        { "id": "text_target00", "type": "text", "position": { "x": 600, "y": 100 },
          "width": 200, "height": 200, "data": { "text": "b" } }
      ],
      "edges": [
        { "id": "text_source00->text_target00",
          "source": "text_source00", "target": "text_target00" }
      ],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const SOURCE: &str = "text_source00";
const EDGE: &str = "text_source00->text_target00";

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let document = CanvasDocument::from_json(WIRED).unwrap();
        let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, workspace.unwrap())
}

/// The source node's screen bounds pin the pane origin: it sits at world (100, 100) with the
/// viewport at identity, so everything else is measured from its top-left corner.
fn source_bounds(cx: &mut TestAppContext, handle: WindowHandle<Root>) -> Bounds<Pixels> {
    cx.update_window(handle.into(), |_, window, _| window.find(SOURCE).bounds())
        .unwrap()
}

/// World (450, 200), the midpoint of the curve, in screen pixels.
fn on_the_curve(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(bounds.origin.x + px(350.0), bounds.origin.y + px(100.0))
}

fn edge_count(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> usize {
    cx.update(|cx| workspace.read(cx).document(cx).read(cx).edges().len())
}

fn selected_edge(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<String> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .selected_edges()
            .iter()
            .map(std::string::ToString::to_string)
            .collect()
    })
}

fn selected_nodes(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> usize {
    cx.update(|cx| workspace.read(cx).document(cx).read(cx).selected().len())
}

#[gpui_kit::test]
fn clicking_a_curve_selects_the_edge_and_clears_the_nodes(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let bounds = source_bounds(cx, handle);

    // Select a node first, so the clear is visible rather than vacuous.
    cx.update_window(handle.into(), |_, window, cx| {
        let header = point(bounds.origin.x + px(100.0), bounds.origin.y + px(12.0));
        window.drag(header, header, cx);
    })
    .unwrap();
    assert_eq!(selected_nodes(cx, &workspace), 1, "the node is selected");

    cx.update_window(handle.into(), |_, window, cx| {
        let at = on_the_curve(bounds);
        window.drag(at, at, cx);
    })
    .unwrap();

    assert_eq!(selected_edge(cx, &workspace), vec![EDGE.to_string()]);
    assert_eq!(
        selected_nodes(cx, &workspace),
        0,
        "picking an edge clears the node selection"
    );
}

#[gpui_kit::test]
fn a_press_beside_the_curve_selects_nothing(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let bounds = source_bounds(cx, handle);

    cx.update_window(handle.into(), |_, window, cx| {
        // Well clear of the interaction width, which is ten world units either side.
        let at = point(bounds.origin.x + px(350.0), bounds.origin.y + px(160.0));
        window.drag(at, at, cx);
    })
    .unwrap();

    assert!(selected_edge(cx, &workspace).is_empty());
    assert_eq!(selected_nodes(cx, &workspace), 0);
}

#[gpui_kit::test]
fn deleting_a_selected_edge_is_undoable(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let bounds = source_bounds(cx, handle);
    assert_eq!(edge_count(cx, &workspace), 1);

    cx.update_window(handle.into(), |_, window, cx| {
        let at = on_the_curve(bounds);
        window.drag(at, at, cx);
        window.press("backspace", cx);
    })
    .unwrap();

    assert_eq!(edge_count(cx, &workspace), 0, "the edge is gone");
    assert_eq!(
        cx.update(|cx| { workspace.read(cx).document(cx).read(cx).nodes().len() }),
        2,
        "and both nodes stayed"
    );

    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-z", cx))
        .unwrap();

    assert_eq!(edge_count(cx, &workspace), 1, "undo brings it back");
    assert!(
        cx.update(|cx| {
            workspace.read(cx).document(cx).read(cx).edges()[0].id == EdgeId::from(EDGE)
        }),
        "the same edge, not a new one"
    );
}
