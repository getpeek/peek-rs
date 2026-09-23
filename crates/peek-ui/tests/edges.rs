//! The edge layer: a press on a curve selects the edge, Delete removes it, undo brings it back,
//! and an option-drag from one card onto another connects them. Driven through real gpui event
//! dispatch.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, TestAppContext, VisualTestContext, WindowHandle, point, px, size,
};
use peek_document::{CanvasDocument, EdgeId, NodeId};
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

fn open(cx: &mut TestAppContext, json: &str) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let document = CanvasDocument::from_json(json).unwrap();
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
    bounds_of(cx, handle, SOURCE)
}

fn bounds_of(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    id: &'static str,
) -> Bounds<Pixels> {
    cx.update_window(handle.into(), |_, window, _| window.find(id).bounds())
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
    let (handle, workspace) = open(cx, WIRED);
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
    let (handle, workspace) = open(cx, WIRED);
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
    let (handle, workspace) = open(cx, WIRED);
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

/// A variable, a query to its right and a text below it, each 200x200 and unconnected. Only
/// variable -> query is a connection the reference accepts.
const UNWIRED: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [
        { "id": "variable_src0", "type": "variable", "position": { "x": 100, "y": 100 },
          "width": 200, "height": 200, "data": { "rows": [{ "name": "id", "value": "1" }] } },
        { "id": "query_target0", "type": "query", "position": { "x": 600, "y": 100 },
          "width": 200, "height": 200, "data": { "query": "select 1" } },
        { "id": "text_bystand0", "type": "text", "position": { "x": 100, "y": 450 },
          "width": 200, "height": 200, "data": { "text": "c" } }
      ],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const VARIABLE: &str = "variable_src0";
const QUERY: &str = "query_target0";
const TEXT: &str = "text_bystand0";

/// A left drag with option held for every event of the gesture. The kit's `Window::drag` sends
/// unmodified events, and option is read from the press.
fn alt_drag(visual: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    let modifiers = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    visual.simulate_event(MouseMoveEvent {
        position: from,
        pressed_button: None,
        modifiers,
    });
    visual.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: from,
        modifiers,
        click_count: 1,
        first_mouse: false,
    });
    visual.simulate_event(MouseMoveEvent {
        position: to,
        pressed_button: Some(MouseButton::Left),
        modifiers,
    });
    visual.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: to,
        modifiers,
        click_count: 1,
    });
}

fn centre(bounds: Bounds<Pixels>) -> Point<Pixels> {
    bounds.center()
}

fn edge_ids(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<String> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .edges()
            .iter()
            .map(|edge| edge.id.to_string())
            .collect()
    })
}

fn position_of(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>, id: &str) -> (f64, f64) {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let node = document
            .read(cx)
            .node(&NodeId::from(id))
            .expect("node exists");
        (node.position.x, node.position.y)
    })
}

#[gpui_kit::test]
fn option_dragging_a_variable_onto_a_query_connects_them(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, UNWIRED);
    let from = centre(bounds_of(cx, handle, VARIABLE));
    let to = centre(bounds_of(cx, handle, QUERY));

    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    alt_drag(&mut visual, from, to);

    assert_eq!(
        edge_ids(cx, &workspace),
        vec![format!("{VARIABLE}->{QUERY}")]
    );
    assert_eq!(
        position_of(cx, &workspace, VARIABLE),
        (100.0, 100.0),
        "the source card stays put"
    );

    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-z", cx))
        .unwrap();
    assert!(edge_ids(cx, &workspace).is_empty(), "undo removes the edge");
}

#[gpui_kit::test]
fn option_dragging_between_kinds_that_cannot_connect_does_nothing(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, UNWIRED);
    let variable = centre(bounds_of(cx, handle, VARIABLE));
    let query = centre(bounds_of(cx, handle, QUERY));
    let text = centre(bounds_of(cx, handle, TEXT));

    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    alt_drag(&mut visual, variable, text);
    alt_drag(&mut visual, text, query);
    alt_drag(&mut visual, query, variable);

    assert!(edge_ids(cx, &workspace).is_empty());
}

#[gpui_kit::test]
fn option_dragging_onto_bare_canvas_does_nothing(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, UNWIRED);
    let bounds = bounds_of(cx, handle, VARIABLE);
    let from = centre(bounds);
    // World (450, 200): the gap between the variable and the query.
    let to = point(bounds.origin.x + px(350.0), bounds.origin.y + px(100.0));

    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    alt_drag(&mut visual, from, to);

    assert!(edge_ids(cx, &workspace).is_empty());
    assert_eq!(position_of(cx, &workspace, VARIABLE), (100.0, 100.0));
}
