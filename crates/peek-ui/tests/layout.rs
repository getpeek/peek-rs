//! `View::Organize`, `View::Schema` and `Zoom::FitSelectionAndLock`, through real dispatch.
//!
//! Every one of these is dispatched the way the command palette dispatches it — through the
//! window, onto whatever holds focus — because that is the failure mode the canvas dispatch
//! modules exist to prevent: a handler that lives on a node view is listed by the palette and
//! then silently does nothing.

use std::time::Duration;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_canvas::Point;
use peek_document::geometry::{Rect, Size as WorldSize};
use peek_document::{CanvasDocument, NodeId, NodeType};
use peek_ui::WorkspaceView;
use peek_ui::commands::actions;

const FIXTURE: &str = include_str!("../../peek-document/tests/fixtures/plock-local.json");

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let document = CanvasDocument::from_json(FIXTURE).unwrap();
        let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, workspace.unwrap())
}

/// The layout ticks on the wall clock, so a test advances it by sleeping and re-rendering.
/// 70 ms is over the four-tick catch-up cap, which is what keeps the sleeping short.
fn run_frames(cx: &mut TestAppContext, handle: WindowHandle<Root>, frames: usize) {
    for _ in 0..frames {
        std::thread::sleep(Duration::from_millis(70));
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
}

fn positions(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<(NodeId, Point)> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .nodes()
            .iter()
            .map(|node| (node.id.clone(), node.position))
            .collect()
    })
}

fn dispatch(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    action: Box<dyn gpui_kit::Action>,
) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_action(action, cx);
    })
    .unwrap();
}

/// The whole point of handling this on the canvas: the palette confirms through the canvas
/// focus handle, and the nodes have to actually move.
#[gpui_kit::test]
fn organizing_the_canvas_moves_the_nodes(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = positions(cx, &workspace);
    assert!(before.len() >= 2, "the fixture has something to arrange");

    dispatch(cx, handle, Box::new(actions::view::Organize));
    run_frames(cx, handle, 4);

    let after = positions(cx, &workspace);
    assert_ne!(before, after, "the simulation ran and wrote its positions");
}

/// `organizeCanvas.tsx` bails on the schema page: that page is laid out by the command that
/// builds it, and a second simulation would fight it over every node.
#[gpui_kit::test]
fn organizing_is_refused_on_the_schema_page(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            document.add_page(Some("schema".to_string()), None);
            for offset in [0.0, 40.0] {
                document.create_node(
                    NodeType::TableDefinition,
                    Rect::new(Point::new(offset, offset), WorldSize::new(450.0, 200.0)),
                );
            }
            cx.notify();
        });
    });
    let before = positions(cx, &workspace);

    dispatch(cx, handle, Box::new(actions::view::Organize));
    run_frames(cx, handle, 4);

    assert_eq!(
        before,
        positions(cx, &workspace),
        "the schema page is left exactly as it was"
    );
}

/// A run started on one page must not go on writing positions after the user has moved to
/// another one — `set_position` writes to whichever page is active.
#[gpui_kit::test]
fn switching_pages_stops_a_running_layout(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    dispatch(cx, handle, Box::new(actions::view::Organize));
    run_frames(cx, handle, 1);

    let arranged = positions(cx, &workspace);
    let origin = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            let origin = document.active_page_id().clone();
            document.add_page(None, None);
            cx.notify();
            origin
        })
    });

    run_frames(cx, handle, 4);

    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            document.switch_page(&origin);
            cx.notify();
        });
    });
    assert_eq!(
        arranged,
        positions(cx, &workspace),
        "the run stopped when the page it started on stopped being active"
    );
}

/// The schema page itself needs a live database, which a headless test has no business
/// opening; what it can pin is that the command reaches the canvas and lands on the page the
/// reference names literally, which is also what `View::Organize` refuses to run on.
#[gpui_kit::test]
fn viewing_the_schema_opens_the_page_it_is_built_on(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);

    dispatch(cx, handle, Box::new(actions::view::Schema));

    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        assert_eq!(document.active_page().name, "schema");
        assert!(document.on_schema_page());
    });
}

/// `fitNodesToView.tsx`'s second command: frame the selection, then hold the camera there.
#[gpui_kit::test]
fn fitting_the_selection_and_locking_frames_it_and_holds_the_camera(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let initial = cx.update(|cx| workspace.read(cx).camera(cx));

    // Far enough from the fixture that no starting camera could already be framing it.
    let placed = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            let id = document.create_node(
                NodeType::Text,
                Rect::new(Point::new(9000.0, 9000.0), WorldSize::new(280.0, 140.0)),
            );
            document.select_only([id.clone()]);
            cx.notify();
            id
        })
    });

    dispatch(cx, handle, Box::new(actions::zoom::FitSelectionAndLock));

    assert!(
        cx.update(|cx| workspace.read(cx).is_camera_locked(cx)),
        "the camera is locked, not toggled"
    );
    let target = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!(!target.approx_eq(initial), "and it framed the selection");
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let bounds = document.read(cx).bounds_of([&placed]).unwrap();
        let visible = target.visible_world_rect(peek_canvas::Size::new(1200.0, 800.0));
        assert!(
            visible.contains(bounds.min()) && visible.contains(bounds.max()),
            "the selected node is in frame: {bounds:?} in {visible:?}"
        );
    });

    // A second press must leave it locked: the reference sets the lock, it does not flip it.
    dispatch(cx, handle, Box::new(actions::zoom::FitSelectionAndLock));
    assert!(cx.update(|cx| workspace.read(cx).is_camera_locked(cx)));
}
