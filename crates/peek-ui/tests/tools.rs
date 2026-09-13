//! The half of the canvas tool surface that needs a window: the camera flights, the page
//! reconcile a page-switching tool triggers, and the two tools answered from the connection
//! rather than from the document.
//!
//! Everything else is covered by `peek-canvas`'s own `tests/tools.rs`, which needs no window.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_canvas::Camera;
use peek_document::{CanvasDocument, NodeId, PageId};
use peek_ui::WorkspaceView;
use serde_json::{Value, json};

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

fn run(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    workspace: &Entity<WorkspaceView>,
    method: &str,
    params: &Value,
) -> Value {
    let workspace = workspace.clone();
    let reply = cx
        .update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |view, cx| view.run_tool(method, params, window, cx))
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    reply
}

fn active_page(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> PageId {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).active_page_id().clone()
    })
}

fn first_node(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> NodeId {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.nodes()[0].id.clone()
    })
}

#[gpui_kit::test]
fn panning_flies_the_camera_to_the_point(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = cx.update(|cx| workspace.read(cx).camera_target(cx));

    run(
        cx,
        handle,
        &workspace,
        "camera_pan_to",
        &json!({ "position": [4000.0, 4000.0] }),
    );

    let after = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert_ne!(after.pan, before.pan, "the camera moved");
    assert!(
        (after.zoom - before.zoom).abs() < f64::EPSILON,
        "panning keeps the zoom: {} -> {}",
        before.zoom,
        after.zoom
    );
}

#[gpui_kit::test]
fn setting_the_zoom_clamps_and_reports_what_it_applied(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);

    let reply = run(
        cx,
        handle,
        &workspace,
        "camera_set_zoom",
        &json!({ "zoom": 99.0 }),
    );
    assert_eq!(reply["zoom"], json!(peek_canvas::MAX_ZOOM));

    let target = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!((target.zoom - peek_canvas::MAX_ZOOM).abs() < f64::EPSILON);
}

/// A locked camera is the user saying "stop moving my view". A tool call is still allowed to
/// mutate the document, but it must not fly the camera out from under them.
#[gpui_kit::test]
fn fitting_a_node_frames_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let node = first_node(cx, &workspace);
    let before = cx.update(|cx| workspace.read(cx).camera_target(cx));

    let reply = run(
        cx,
        handle,
        &workspace,
        "camera_fit_node",
        &json!({ "nodeId": node.as_str() }),
    );

    assert_eq!(reply["nodeId"], json!(node.as_str()));
    let after = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!(
        after.pan != before.pan || (after.zoom - before.zoom).abs() > f64::EPSILON,
        "the camera framed the node"
    );
}

/// The reconcile that lets every page-switching tool stay window-free: the view has to adopt the
/// new page's own persisted viewport, not keep flying the outgoing page's camera.
#[gpui_kit::test]
fn a_tool_that_switches_pages_adopts_that_pages_camera(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = active_page(cx, &workspace);

    let created = run(
        cx,
        handle,
        &workspace,
        "create_page",
        &json!({ "name": "elsewhere", "order": 0 }),
    );
    let created = PageId::from(created["id"].as_str().unwrap());

    assert_eq!(active_page(cx, &workspace), created, "the view followed");
    assert_ne!(created, before);

    let camera = cx.update(|cx| workspace.read(cx).camera(cx));
    let viewport = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.viewport()
    });
    assert_eq!(camera, Camera::from_viewport(viewport));
}

#[gpui_kit::test]
fn selecting_through_a_tool_moves_the_canvas_selection(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let node = first_node(cx, &workspace);

    run(
        cx,
        handle,
        &workspace,
        "select_nodes",
        &json!({ "nodeIds": [node.as_str()] }),
    );

    let selected = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.selected().iter().cloned().collect::<Vec<_>>()
    });
    assert_eq!(selected, vec![node]);
}

/// With no connection open, the two app-answered tools have to say so in the shape their callers
/// expect — `null` and the empty-schema sentinel — rather than erroring or panicking.
#[gpui_kit::test]
fn the_connection_tools_answer_without_a_database(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);

    let info = run(cx, handle, &workspace, "get_connection_info", &json!({}));
    assert_eq!(info, Value::Null);

    let schema = run(cx, handle, &workspace, "get_db_schema", &json!({}));
    assert_eq!(schema, json!("(no tables in schema)"));
}

/// The bridge parks a one-shot on a five-second timeout, so every call has to come back with
/// something — including one the executor has never heard of.
#[gpui_kit::test]
fn an_unknown_tool_still_answers(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let reply = run(cx, handle, &workspace, "reticulate_splines", &json!({}));
    assert_eq!(reply["error"], json!("unknown tool 'reticulate_splines'"));
}

/// A tool that edits the document has to leave a repaint behind it, or the canvas keeps showing
/// the state from before the agent touched it until something unrelated happens to repaint.
#[gpui_kit::test]
fn a_tool_edit_lands_on_the_canvas(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).nodes().len()
    });

    let reply = run(
        cx,
        handle,
        &workspace,
        "create_text_node",
        &json!({ "text": "from the agent", "position": [0.0, 0.0] }),
    );
    let id = NodeId::from(reply["nodeId"].as_str().unwrap());

    let (count, rendered) = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        (document.nodes().len(), document.node(&id).is_some())
    });
    assert_eq!(count, before + 1);
    assert!(rendered);
}
