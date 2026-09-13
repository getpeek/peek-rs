//! Canvas keyboard navigation: jump labels, directional selection and enter-to-edit, driven
//! through real gpui key dispatch in a headless window.

use std::time::Duration;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_canvas::{Document, JumpMode, Point};
use peek_document::{CanvasDocument, NodeId, NodeKind};
use peek_ui::WorkspaceView;

const FIXTURE: &str = include_str!("../../peek-document/tests/fixtures/plock-local.json");

/// The fixture's active page holds two nodes: this query, off to the left of the viewport, and
/// the result it produced, which is what the camera opens on.
const QUERY: &str = "query_U96W6P0U";

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
    (handle, workspace.unwrap())
}

fn settle(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    std::thread::sleep(Duration::from_millis(350));
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn press(cx: &mut TestAppContext, handle: WindowHandle<Root>, keys: &[&str]) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        for key in keys {
            window.press(key, cx);
            // Each press has to see the context the previous one left behind: jump mode only
            // takes letters out of the tool bindings' hands from the next painted frame on.
            window.render_frame(cx);
        }
    })
    .unwrap();
}

fn document(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Entity<Document> {
    cx.update(|cx| workspace.read(cx).document(cx))
}

fn node_count(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> usize {
    let document = document(cx, workspace);
    cx.update(|cx| document.read(cx).nodes().len())
}

fn selected(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<NodeId> {
    let document = document(cx, workspace);
    cx.update(|cx| document.read(cx).selected().iter().cloned().collect())
}

/// The labels the overlay is showing, rebuilt from the same inputs the view uses, so a test can
/// name the node behind a label without reaching into the view's private state.
fn labels(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> JumpMode {
    let document = document(cx, workspace);
    let camera = cx.update(|cx| workspace.read(cx).camera(cx));
    let pane = peek_canvas::Size::new(1200.0, 800.0);
    let visible = camera.visible_world_rect(pane);
    let centre = camera.screen_to_world(Point::new(600.0, 400.0));
    cx.update(|cx| JumpMode::new(document.read(cx).nodes(), visible, centre))
        .unwrap()
}

fn select_query(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> NodeId {
    let document = document(cx, workspace);
    cx.update(|cx| {
        document.update(cx, |document, _| {
            let id = document
                .nodes()
                .iter()
                .find(|node| matches!(node.kind, NodeKind::Query(_)))
                .expect("the fixture has a query node")
                .id
                .clone();
            document.select_only([id.clone()]);
            id
        })
    })
}

fn canvas_focused(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    workspace: &Entity<WorkspaceView>,
) -> bool {
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.read(cx).canvas_focus().is_focused(window)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_label_selects_its_node_and_flies_to_it_at_full_zoom(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let expected = labels(cx, &workspace).targets()[0].id.clone();

    press(cx, handle, &["g", "a"]);

    assert_eq!(selected(cx, &workspace), vec![expected]);
    let target = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!(
        (target.zoom - 1.0).abs() < f64::EPSILON,
        "a jump always lands at 100%: {}",
        target.zoom
    );
    settle(cx, handle);
}

#[gpui_kit::test]
fn jump_mode_takes_the_tool_keys_out_of_play(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = node_count(cx, &workspace);

    // `q` arms the query tool on a plain canvas. Under `CANVAS_JUMPING` that binding is dead,
    // so this is a label keystroke and nothing is placed.
    press(cx, handle, &["g", "q"]);

    assert_eq!(
        node_count(cx, &workspace),
        before,
        "jump mode must not arm the query tool"
    );
}

#[gpui_kit::test]
fn backspace_in_jump_mode_does_not_delete_the_selection(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = node_count(cx, &workspace);
    select_query(cx, &workspace);

    press(cx, handle, &["g", "backspace"]);

    assert_eq!(
        node_count(cx, &workspace),
        before,
        "backspace edits the label, never the document"
    );
}

#[gpui_kit::test]
fn escape_leaves_jump_mode_with_the_selection_intact(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let chosen = select_query(cx, &workspace);

    press(cx, handle, &["g", "escape"]);
    assert_eq!(
        selected(cx, &workspace),
        vec![chosen],
        "the first escape only cancels the jump"
    );

    // Jump mode is gone, so a second escape reaches its usual handler.
    press(cx, handle, &["escape"]);
    assert!(selected(cx, &workspace).is_empty());
}

#[gpui_kit::test]
fn a_cancelled_jump_can_be_reopened(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let expected = labels(cx, &workspace).targets()[0].id.clone();
    let before = selected(cx, &workspace);

    // A non-letter cancels. The mode has to be genuinely reset afterwards, not wedged, and
    // cancelling must not disturb the selection.
    press(cx, handle, &["g", "1"]);
    assert_eq!(selected(cx, &workspace), before);

    press(cx, handle, &["g", "a"]);
    assert_eq!(selected(cx, &workspace), vec![expected]);
    settle(cx, handle);
}

#[gpui_kit::test]
fn meta_arrows_walk_the_selection_without_changing_the_zoom(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let zoom = cx.update(|cx| workspace.read(cx).camera(cx)).zoom;

    // Nothing is selected, so the first press ignores the direction entirely and anchors on
    // the node nearest the viewport centre — the result node.
    press(cx, handle, &["cmd-left"]);
    let anchored = selected(cx, &workspace);
    assert_eq!(anchored.len(), 1, "the first press picks an anchor");
    settle(cx, handle);

    // The query sits to the left of that anchor, so the second press steps onto it.
    press(cx, handle, &["cmd-left"]);
    assert_eq!(
        selected(cx, &workspace),
        vec![NodeId::from(QUERY)],
        "stepped from {anchored:?}"
    );

    let target = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!(
        (target.zoom - zoom).abs() < f64::EPSILON,
        "arrow navigation preserves the zoom: {zoom} -> {}",
        target.zoom
    );
    settle(cx, handle);
}

#[gpui_kit::test]
fn an_arrow_with_nothing_in_the_way_leaves_the_selection_alone(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);

    press(cx, handle, &["cmd-right"]);
    let anchored = selected(cx, &workspace);
    assert_eq!(anchored.len(), 1);
    settle(cx, handle);

    // Nothing lies right of the anchor, so the cone comes up empty. That is a no-op, not a
    // deselect.
    press(cx, handle, &["cmd-right"]);
    assert_eq!(selected(cx, &workspace), anchored);
}

#[gpui_kit::test]
fn enter_focuses_the_selected_querys_editor_and_escape_gives_focus_back(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);

    // Walk to the query rather than selecting it outright: escape is handled by the node's own
    // card, which only exists once the node is on screen.
    press(cx, handle, &["cmd-left"]);
    settle(cx, handle);
    press(cx, handle, &["cmd-left"]);
    settle(cx, handle);
    assert_eq!(selected(cx, &workspace), vec![NodeId::from(QUERY)]);

    press(cx, handle, &["enter"]);
    assert!(
        !canvas_focused(cx, handle, &workspace),
        "enter hands focus to the query editor, off the canvas"
    );

    press(cx, handle, &["escape"]);
    assert!(
        canvas_focused(cx, handle, &workspace),
        "escape hands focus back to the canvas"
    );
}

#[gpui_kit::test]
fn enter_does_nothing_without_exactly_one_selected_query(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);

    press(cx, handle, &["cmd-a", "enter"]);

    assert!(
        canvas_focused(cx, handle, &workspace),
        "a multi-node selection is not one query to edit"
    );
}
