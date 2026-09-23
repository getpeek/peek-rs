//! Version history, driven through real key and pointer dispatch: the timeline opens on the
//! present, scrubbing shows a past version without touching the document, and a restore is a
//! labelled checkpoint and one undo step.
//!
//! The workspace a test builds has no history file, so every checkpoint here lives in memory;
//! the log format and its disk handling are covered in `peek-document` and `canvas::history`.

use std::time::Duration;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_canvas::Document;
use peek_document::{CanvasDocument, NodeId};
use peek_ui::WorkspaceView;
use peek_ui::commands::actions;

const FIXTURE: &str = include_str!("../../peek-document/tests/fixtures/plock-local.json");
/// A page with a handful of nodes on it.
const PAGE: &str = "page_hbnrnVts";
/// Deleted between two versions, so its presence on screen says which version is showing.
const WITNESS: &str = "query_Iot82jq3";

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
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("page-tab-{PAGE}"), cx);
        window.render_frame(cx);
    })
    .unwrap();
    (handle, workspace.unwrap())
}

fn document(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Entity<Document> {
    cx.update(|cx| workspace.read(cx).document(cx))
}

fn has_witness(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> bool {
    let document = document(cx, workspace);
    cx.update(|cx| document.read(cx).node(&NodeId::from(WITNESS)).is_some())
}

fn toggle_history(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_action(Box::new(actions::history::Toggle), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, handle: WindowHandle<Root>, key: &str) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.press(key, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Lets the camera flight the timeline starts on every selection land. Flights run on the wall
/// clock, as `navigation.rs` explains.
fn settle(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    std::thread::sleep(Duration::from_millis(450));
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn on_screen(cx: &mut TestAppContext, handle: WindowHandle<Root>, id: &str) -> bool {
    let id = id.to_string();
    cx.update_window(handle.into(), |_, window, _| window.try_find(id).is_some())
        .unwrap()
}

/// Two versions of the page: as loaded, and with the witness deleted.
fn two_versions(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    let (handle, workspace) = open(cx);
    toggle_history(cx, handle);
    press(cx, handle, "escape");
    let document = document(cx, &workspace);
    document.update(cx, |document, cx| {
        document.remove_nodes(&[NodeId::from(WITNESS)]);
        document.checkpoint();
        cx.notify();
    });
    toggle_history(cx, handle);
    (handle, workspace)
}

#[gpui_kit::test]
fn the_timeline_opens_on_the_present_and_escape_hands_focus_back(cx: &mut TestAppContext) {
    let (handle, _workspace) = open(cx);
    toggle_history(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("history-panel").is_some());
        assert!(
            window.try_find("history-card").is_some(),
            "the present is selected"
        );
        assert!(window.try_find("history-preview").is_none());
    })
    .unwrap();

    press(cx, handle, "escape");
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("history-panel").is_none());
        assert_eq!(window.find("canvas").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn scrubbing_back_shows_the_old_version_and_leaves_the_document_alone(cx: &mut TestAppContext) {
    let (handle, workspace) = two_versions(cx);
    let document = document(cx, &workspace);
    // Content rather than the revision: framing the version moves the camera, and the viewport
    // is persisted.
    let content = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let document = document.read(cx);
            document.page_snapshot(document.active_page_id())
        })
    };
    let before = content(cx);

    press(cx, handle, "left");
    settle(cx, handle);
    assert!(on_screen(cx, handle, "history-preview"));
    assert!(
        on_screen(cx, handle, WITNESS),
        "the old version still has it"
    );
    assert!(!has_witness(cx, &workspace), "the document does not");
    assert_eq!(content(cx), before);

    // Canvas bindings are dead under the timeline: Backspace would otherwise delete this.
    document.update(cx, |document, cx| {
        document.select_only([NodeId::from("query_TVz-Cznu")]);
        cx.notify();
    });
    press(cx, handle, "backspace");
    assert_eq!(content(cx), before);

    press(cx, handle, "escape");
    settle(cx, handle);
    assert!(
        !on_screen(cx, handle, "history-preview"),
        "back to the present"
    );
    assert!(!on_screen(cx, handle, WITNESS));
    assert!(
        on_screen(cx, handle, "history-panel"),
        "the first Escape only steps back"
    );
}

#[gpui_kit::test]
fn restoring_brings_the_version_back_as_one_undo_step(cx: &mut TestAppContext) {
    let (handle, workspace) = two_versions(cx);
    press(cx, handle, "left");
    press(cx, handle, "enter");
    settle(cx, handle);

    assert!(has_witness(cx, &workspace));
    assert!(on_screen(cx, handle, "history-toast"));
    assert!(!on_screen(cx, handle, "history-preview"));

    press(cx, handle, "escape");
    press(cx, handle, "cmd-z");
    assert!(
        !has_witness(cx, &workspace),
        "the restore undoes in one press"
    );
}

#[gpui_kit::test]
fn the_palette_closes_the_timeline(cx: &mut TestAppContext) {
    let (handle, _workspace) = two_versions(cx);
    press(cx, handle, "left");
    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_action(Box::new(actions::command_palette::Open), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert!(!on_screen(cx, handle, "history-panel"));
    assert!(!on_screen(cx, handle, "history-preview"));
}

#[gpui_kit::test]
fn cmd_y_opens_the_timeline_and_closes_it_from_a_preview(cx: &mut TestAppContext) {
    let (handle, _workspace) = two_versions(cx);
    press(cx, handle, "escape");
    assert!(!on_screen(cx, handle, "history-panel"));

    press(cx, handle, "cmd-y");
    assert!(on_screen(cx, handle, "history-panel"));
    press(cx, handle, "left");
    assert!(on_screen(cx, handle, "history-preview"));

    press(cx, handle, "cmd-y");
    assert!(!on_screen(cx, handle, "history-panel"));
    assert!(!on_screen(cx, handle, "history-preview"));
    cx.update_window(handle.into(), |_, window, _| {
        assert_eq!(window.find("canvas").focused(), Some(true));
    })
    .unwrap();
}
