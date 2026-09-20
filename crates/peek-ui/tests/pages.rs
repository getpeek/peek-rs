//! Page tabs in the title bar: switching, adding, closing and renaming, all through real
//! pointer and key dispatch.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_canvas::Camera;
use peek_document::{CanvasDocument, PageId};
use peek_ui::WorkspaceView;

const FIXTURE: &str = include_str!("../../peek-document/tests/fixtures/plock-local.json");

fn open_with(
    cx: &mut TestAppContext,
    document: CanvasDocument,
) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, workspace.unwrap())
}

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    open_with(cx, CanvasDocument::from_json(FIXTURE).unwrap())
}

fn pages(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<PageId> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.pages().map(|page| page.id.clone()).collect()
    })
}

fn active(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> PageId {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).active_page_id().clone()
    })
}

fn active_name(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> String {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).active_page().name.clone()
    })
}

fn tab(id: &PageId) -> String {
    format!("page-tab-{id}")
}

#[gpui_kit::test]
fn clicking_a_tab_switches_pages_and_the_camera_follows(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let ids = pages(cx, &workspace);
    let target = ids
        .iter()
        .find(|id| *id != &active(cx, &workspace))
        .expect("the fixture has more than one page")
        .clone();

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(tab(&target), cx);
    })
    .unwrap();

    assert_eq!(active(cx, &workspace), target);
    // The camera must adopt the page's stored viewport, not merely the id.
    let (camera, viewport) = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        (workspace.read(cx).camera(cx), document.read(cx).viewport())
    });
    assert_eq!(camera, Camera::from_viewport(viewport));
}

#[gpui_kit::test]
fn the_add_button_appends_a_page_and_activates_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = pages(cx, &workspace).len();

    cx.update_window(handle.into(), |_, window, cx| window.click("page-add", cx))
        .unwrap();

    let after = pages(cx, &workspace);
    assert_eq!(after.len(), before + 1);
    assert_eq!(active(cx, &workspace), *after.last().unwrap());
    assert_eq!(active_name(cx, &workspace), format!("Page {}", before + 1));
}

#[gpui_kit::test]
fn the_close_button_is_only_on_the_active_tab(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let active_id = active(cx, &workspace);
    let other = pages(cx, &workspace)
        .into_iter()
        .find(|id| id != &active_id)
        .expect("more than one page");

    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find(format!("page-close-{active_id}")).is_some());
        assert!(
            window.try_find(format!("page-close-{other}")).is_none(),
            "an inactive tab has no close button"
        );
        assert!(
            window.try_find(tab(&other)).is_some(),
            "but the tab itself is still rendered"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_lone_page_cannot_be_closed(cx: &mut TestAppContext) {
    let (handle, workspace) = open_with(cx, CanvasDocument::empty());
    let only = active(cx, &workspace);

    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find(tab(&only)).is_some());
        assert!(window.try_find(format!("page-close-{only}")).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_an_empty_page_skips_the_confirmation(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = pages(cx, &workspace).len();

    // `cmd-t` makes a page with no nodes; there is nothing to lose, so it goes at once.
    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-t", cx))
        .unwrap();
    let fresh = active(cx, &workspace);
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(format!("page-close-{fresh}"), cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(pages(cx, &workspace).len(), before);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("close-page-confirm").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_a_populated_page_asks_first(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = pages(cx, &workspace).len();
    let doomed = active(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(format!("page-close-{doomed}"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("close-page-confirm").is_some());
    })
    .unwrap();
    assert_eq!(
        pages(cx, &workspace).len(),
        before,
        "nothing is deleted until it is confirmed"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("close-page-delete", cx);
    })
    .unwrap();
    cx.run_until_parked();

    let after = pages(cx, &workspace);
    assert_eq!(after.len(), before - 1);
    assert!(!after.contains(&doomed));
}

#[gpui_kit::test]
fn double_click_renames_a_page(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let target = active(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.double_click(tab(&target), cx);
        window.render_frame(cx);
        // The field opens select-all, so typing replaces rather than appends.
        window.input("Analytics", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(active_name(cx, &workspace), "Analytics");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("page-rename").is_none());
    })
    .unwrap();
}

/// The tab must stay a tab while it is being renamed. It regressed into an invisible sliver:
/// the editing element carried a `max_w` and no width of its own, and an `InputState` measures
/// its content at zero, so the flex item collapsed to its padding until enter restored the pill.
#[gpui_kit::test]
fn a_tab_being_renamed_keeps_the_width_of_a_tab(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let target = active(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        let before = window.find(tab(&target)).bounds();
        window.double_click(tab(&target), cx);
        window.render_frame(cx);

        let editing = window.find("page-rename").bounds();
        assert!(
            editing.size.width > before.size.width / 2.0,
            "the editing tab is {:?} wide against the pill's {:?}",
            editing.size.width,
            before.size.width
        );
        assert!(editing.size.height >= before.size.height);
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_cancels_a_rename_and_hands_focus_back(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let target = active(cx, &workspace);
    let original = active_name(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.double_click(tab(&target), cx);
        window.render_frame(cx);
        window.input("discarded", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(active_name(cx, &workspace), original);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("page-rename").is_none());
        // The failure the Text node hit: a dead focus handle leaves the window focused on
        // nothing and every canvas binding silently stops working.
        assert_eq!(window.find("canvas").focused(), Some(true));
    })
    .unwrap();
}
