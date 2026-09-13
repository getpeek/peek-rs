//! The title-bar connection picker: that the pill is there, and that choosing another connection
//! rebuilds the window around its document.
//!
//! Everything here runs in [`PersistenceMode::ReadOnly`], the default for
//! `WorkspaceView::with_document`, in which `DocumentStore` creates no directories and writes no
//! files — so switching to a connection that does not exist reads `~/peek` and touches nothing.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_document::CanvasDocument;
use peek_ui::WorkspaceView;

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

fn node_count(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> usize {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).nodes().len()
    })
}

fn switch(
    handle: WindowHandle<Root>,
    workspace: &Entity<WorkspaceView>,
    to: (&str, &str),
    cx: &mut TestAppContext,
) {
    let workspace = workspace.clone();
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |view, cx| {
            view.switch_connection(to.0.to_string(), to.1.to_string(), window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
}

/// With no workspaces in the config the pill still renders — an empty `settings.json` is a state,
/// not an absence — which is also the state every other UI test opens in.
#[gpui_kit::test]
fn the_picker_renders_with_an_empty_config(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("connection-picker").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_picker_hides_with_the_rest_of_the_interface(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("cmd-.", cx);
        window.render_frame(cx);
        assert!(window.try_find("connection-picker").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn switching_loads_the_other_connections_document(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    assert!(node_count(cx, &workspace) > 0, "the fixture has nodes");

    switch(handle, &workspace, ("no-such-workspace", "no-such-db"), cx);

    assert_eq!(
        node_count(cx, &workspace),
        0,
        "a connection with no document on disk opens empty rather than keeping the old one"
    );
    assert_eq!(
        cx.update(|cx| {
            let view = workspace.read(cx);
            let (workspace_name, connection) = view.open_connection();
            (workspace_name.to_string(), connection.to_string())
        }),
        ("no-such-workspace".to_string(), "no-such-db".to_string()),
    );
}

/// The canvas is rebuilt around the new document, so the chrome must still dispatch through a
/// live focus handle afterwards — the reason `switch_connection` reuses the old one.
#[gpui_kit::test]
fn the_toolbar_still_works_after_a_switch(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    switch(handle, &workspace, ("no-such-workspace", "no-such-db"), cx);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("Tool::Text", cx);
        window.render_frame(cx);
        let at = gpui_kit::point(px(600.0), px(500.0));
        window.drag(at, at, cx);
    })
    .unwrap();

    assert_eq!(
        node_count(cx, &workspace),
        1,
        "the toolbar armed the tool and the canvas placed into the new document"
    );
}

#[gpui_kit::test]
fn switching_to_the_open_connection_is_a_no_op(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    switch(handle, &workspace, ("ws", "db"), cx);

    let document = cx.update(|cx| workspace.read(cx).document(cx));
    switch(handle, &workspace, ("ws", "db"), cx);

    assert_eq!(
        document,
        cx.update(|cx| workspace.read(cx).document(cx)),
        "re-picking the open connection must not reload it and lose the session's selection"
    );
}

/// The pill owns a dropdown, so a click must actually open one. The menu takes focus off the
/// canvas when it opens and escape must hand it back — the overlay contract, and the only part of
/// the popup this harness can observe: the popover's own elements are not registered for lookup.
#[gpui_kit::test]
fn the_menu_takes_focus_on_open_and_escape_returns_it(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    cx.update_window(handle.into(), |_, window, _| {
        assert_eq!(
            window.find("canvas").focused(),
            Some(true),
            "the canvas starts focused"
        );
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("connection-picker", cx);
        window.render_frame(cx);
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, _| {
        assert_eq!(
            window.find("canvas").focused(),
            Some(false),
            "the menu opened and took focus"
        );
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, _| {
        assert_eq!(
            window.find("canvas").focused(),
            Some(true),
            "escape dismissed the menu and gave focus back"
        );
    })
    .unwrap();
}
