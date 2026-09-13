//! The draw tool: arming, committing a stroke, staying armed for the next one, and drawing
//! straight over a node instead of dragging it. Driven through real pointer dispatch.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, Pixels, Point, TestAppContext, WindowHandle, point, px, size};
use peek_document::geometry::Point as World;
use peek_document::{CanvasDocument, DrawData, NodeId, NodeKind};
use peek_ui::WorkspaceView;

/// One text node at world (100, 100) with an identity viewport, so the only difference between
/// world and window coordinates is the pane's own origin — which [`pane_origin`] recovers.
const ONE_NODE: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "text_aaaaaaaa",
        "type": "text",
        "position": { "x": 100, "y": 100 },
        "width": 300,
        "height": 200,
        "data": { "text": "hi" }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const TEXT: &str = "text_aaaaaaaa";

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let document = CanvasDocument::from_json(ONE_NODE).unwrap();
        let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, workspace.unwrap())
}

/// Where world (0, 0) sits in the window. The viewport is the identity and the zoom is 1, so
/// the text node's window origin minus its world position is the pane's offset.
fn pane_origin(cx: &mut TestAppContext, handle: WindowHandle<Root>) -> Point<Pixels> {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find(TEXT).bounds();
        point(bounds.origin.x - px(100.0), bounds.origin.y - px(100.0))
    })
    .unwrap()
}

fn drawings(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<(NodeId, DrawData)> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document
            .read(cx)
            .nodes()
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Draw(data) => Some((node.id.clone(), data.clone())),
                _ => None,
            })
            .collect()
    })
}

/// Drags in world coordinates. `TestWindowExt::drag` presses at `from`, sends eight interpolated
/// moves and releases at `to`, so a stroke arrives with nine samples.
fn stroke(cx: &mut TestAppContext, handle: WindowHandle<Root>, from: World, to: World) {
    let origin = pane_origin(cx, handle);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "test coordinates are small whole numbers"
    )]
    let at = |world: World| point(origin.x + px(world.x as f32), origin.y + px(world.y as f32));
    cx.update_window(handle.into(), |_, window, cx| {
        window.drag(at(from), at(to), cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_draw_tool_commits_a_stroke_into_a_padded_node(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| window.press("d", cx))
        .unwrap();

    stroke(
        cx,
        handle,
        World::new(600.0, 400.0),
        World::new(700.0, 450.0),
    );

    let committed = drawings(cx, &workspace);
    let [(id, data)] = committed.as_slice() else {
        panic!("expected one drawing, got {committed:?}");
    };
    assert!(id.as_str().starts_with("draw_"));
    assert_eq!(data.points.len(), 9, "the press plus eight moves");
    assert!((data.stroke_width - 4.0).abs() < f64::EPSILON);
    assert_eq!(data.color, "var(--pk-fg)", "not the palette's white");

    let min_x = data.points.iter().fold(f64::MAX, |min, p| min.min(p[0]));
    let min_y = data.points.iter().fold(f64::MAX, |min, p| min.min(p[1]));
    assert!(
        (min_x - 8.0).abs() < 0.01 && (min_y - 8.0).abs() < 0.01,
        "points are relative to the origin and inset by the padding: {min_x}, {min_y}"
    );

    let bounds = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.node(id).expect("the node is on the page").bounds()
    });
    assert_eq!(bounds.origin, World::new(592.0, 392.0));
    assert!(
        (bounds.size.width - 116.0).abs() < 0.01 && (bounds.size.height - 66.0).abs() < 0.01,
        "the 100 x 50 drag inset by 8 on every side: {:?}",
        bounds.size
    );
}

#[gpui_kit::test]
fn the_draw_tool_stays_armed_until_escape(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| window.press("d", cx))
        .unwrap();

    stroke(
        cx,
        handle,
        World::new(600.0, 400.0),
        World::new(660.0, 430.0),
    );
    stroke(
        cx,
        handle,
        World::new(600.0, 500.0),
        World::new(660.0, 530.0),
    );
    assert_eq!(
        drawings(cx, &workspace).len(),
        2,
        "a committed stroke leaves the tool armed: `useDrawTool` never clears place mode"
    );

    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    stroke(
        cx,
        handle,
        World::new(600.0, 600.0),
        World::new(660.0, 630.0),
    );
    assert_eq!(
        drawings(cx, &workspace).len(),
        2,
        "escape disarms it, and the third drag is an ordinary marquee"
    );
}

#[gpui_kit::test]
fn a_stroke_over_a_node_draws_instead_of_dragging_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let text = NodeId::from(TEXT);
    let before = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.node(&text).expect("the fixture node").position
    });

    cx.update_window(handle.into(), |_, window, cx| window.press("d", cx))
        .unwrap();
    // Straight across the text node's header, which is its drag handle when the pen is away.
    stroke(
        cx,
        handle,
        World::new(150.0, 120.0),
        World::new(350.0, 260.0),
    );

    assert_eq!(
        drawings(cx, &workspace).len(),
        1,
        "the stroke was committed"
    );
    let after = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.node(&text).expect("the fixture node").position
    });
    assert_eq!(after, before, "and the node it was drawn over never moved");
}
