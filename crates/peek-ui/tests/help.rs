//! The commands the canvas owns that no node does: the keymap reference, About, the title-bar
//! preference, and the node clipboard behind cut, copy and paste.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, SharedString, TestAppContext, VisualTestContext, WindowHandle,
    point, px, size,
};
use peek_document::geometry::Size;
use peek_document::{CanvasDocument, Cell, Column, NodeId, ResultSet};
use peek_ui::WorkspaceView;
use peek_ui::commands::actions;

const FIXTURE: &str = include_str!("../../peek-document/tests/fixtures/plock-local.json");
/// The window, in the two types that need it: `px` takes `f32`, `Size` is `f64`. Written
/// this way round so the `f32` literals are the source and the widening is lossless, rather
/// than casting `f64` down and tripping `clippy::cast_possible_truncation`.
const WINDOW_WIDTH: f32 = 1200.0;
const WINDOW_HEIGHT: f32 = 800.0;
const WINDOW: Size = Size {
    width: WINDOW_WIDTH as f64,
    height: WINDOW_HEIGHT as f64,
};

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)), |window, cx| {
        let document = CanvasDocument::from_json(FIXTURE).unwrap();
        let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    (handle, workspace.unwrap())
}

/// One result node with two columns and two rows, at the geometry `node/result/mod.rs`'s own
/// tests use — the cell arithmetic in [`cell_at`] is theirs and depends on the 600 px width.
const RESULT_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "query_aaaaaaaa-result-0",
        "type": "result",
        "position": { "x": 40, "y": 120 },
        "width": 600,
        "height": 440,
        "data": { "query": "select * from users" }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

fn result_node() -> NodeId {
    NodeId::from("query_aaaaaaaa-result-0")
}

fn open_result(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)), |window, cx| {
        let document = CanvasDocument::from_json(RESULT_DOCUMENT).unwrap();
        let view = cx.new(|cx| {
            let view = WorkspaceView::with_document("test", document, window, cx);
            view.document(cx).update(cx, |document, _| {
                document.set_result(
                    result_node(),
                    ResultSet::new(
                        vec![Column::new("id", "INT4"), Column::new("name", "VARCHAR")],
                        vec![
                            vec![Cell::Int(0), Cell::Text("row 0".to_string())],
                            vec![Cell::Int(1), Cell::Text("row 1".to_string())],
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

fn table_bounds(cx: &mut TestAppContext, handle: WindowHandle<Root>) -> Bounds<Pixels> {
    cx.update_window(handle.into(), |_, window, _| {
        window
            .find(SharedString::from(format!("{}-table", result_node())))
            .bounds()
    })
    .unwrap()
}

/// The centre of one cell, copied from `node/result/mod.rs`'s own `cell_at`: two columns share
/// the 598 px body, and down the page sit the toolbar (28), the column header (34), then rows.
fn cell_at(bounds: Bounds<Pixels>, row: f32, column: f32) -> Point<Pixels> {
    point(
        bounds.origin.x + px(40.0) + px(column * 299.0),
        bounds.origin.y + px(28.0 + 34.0 + 17.0) + px(row * 34.0),
    )
}

/// A press and release at one point. `Window::click` targets an element by id, which would
/// sidestep the very dispatch this is about.
fn click_at(visual: &mut VisualTestContext, at: Point<Pixels>) {
    visual.simulate_event(MouseMoveEvent {
        position: at,
        pressed_button: None,
        modifiers: Modifiers::default(),
    });
    visual.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    visual.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
}

/// Dispatch is deferred, so nothing has happened until the app parks.
fn run(handle: WindowHandle<Root>, action: impl gpui_kit::Action, cx: &mut TestAppContext) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(action), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn press(handle: WindowHandle<Root>, keys: &[&str], cx: &mut TestAppContext) {
    cx.update_window(handle.into(), |_, window, cx| {
        for key in keys {
            window.press(key, cx);
        }
    })
    .unwrap();
}

fn node_ids(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<String> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .document(cx)
            .read(cx)
            .nodes()
            .iter()
            .map(|node| node.id.to_string())
            .collect()
    })
}

#[gpui_kit::test]
fn the_keymap_shortcut_opens_the_reference_and_escape_closes_it(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    press(handle, &["cmd-/"], cx);
    cx.run_until_parked();

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("keymap-help").is_some(),
            "cmd-/ puts the reference on screen"
        );
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("keymap-help").is_none(),
            "escape closes the topmost overlay"
        );
    })
    .unwrap();
}

/// The palette confirms through the canvas focus handle, which sits above the node views and
/// below the window: a handler anywhere else is listed and then does nothing.
#[gpui_kit::test]
fn about_opens_from_the_action_the_palette_dispatches(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    run(handle, actions::app::About, cx);

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("about").is_some());
    })
    .unwrap();
}

/// A toggle is named for what pressing it does, so the palette row has to flip with the
/// setting — otherwise it is a title with extra steps.
#[test]
fn the_palette_row_names_what_pressing_it_does() {
    let command = peek_ui::commands::find("Settings::ToggleCommandPaletteButton")
        .expect("the command is registered");
    let shown = peek_canvas::Scope::default();
    let hidden = peek_canvas::Scope {
        settings: peek_canvas::SettingsScope {
            palette_button_hidden: true,
            ..peek_canvas::SettingsScope::default()
        },
        ..peek_canvas::Scope::default()
    };
    assert_eq!(command.label(&shown), "Hide command palette button");
    assert_eq!(command.label(&hidden), "Show command palette button");
}

#[gpui_kit::test]
fn the_setting_hides_the_palette_button_and_puts_it_back(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("command-palette").is_some(),
            "the title bar shows the button by default"
        );
    })
    .unwrap();

    run(handle, actions::settings::ToggleCommandPaletteButton, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("command-palette").is_none());
    })
    .unwrap();

    run(handle, actions::settings::ToggleCommandPaletteButton, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("command-palette").is_some(),
            "and the toggle goes both ways"
        );
    })
    .unwrap();
}

/// Cut takes the nodes off the page and paste puts copies back. New ids: pasting under the id
/// it was cut from would collide the moment the same clipboard is pasted twice.
#[gpui_kit::test]
fn cut_takes_the_nodes_and_paste_brings_copies_back(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = node_ids(cx, &workspace);
    assert!(before.len() > 1, "the fixture has something to cut");

    press(handle, &["cmd-a", "cmd-x"], cx);
    assert!(node_ids(cx, &workspace).is_empty(), "the page is empty");

    press(handle, &["cmd-v"], cx);
    let after = node_ids(cx, &workspace);
    assert_eq!(after.len(), before.len(), "every node came back");
    assert!(
        after.iter().all(|id| !before.contains(id)),
        "and none of them reused an id it was cut under"
    );
}

/// Copy leaves the page alone, so pasting after it duplicates rather than restores.
#[gpui_kit::test]
fn copy_then_paste_duplicates_the_selection(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = node_ids(cx, &workspace).len();

    press(handle, &["cmd-a", "cmd-c", "cmd-v"], cx);

    assert_eq!(node_ids(cx, &workspace).len(), before * 2);
}

/// `pasteTranslation.ts`: a paste lands in the middle of what the camera is looking at, not
/// back at the coordinates it was copied from — which may be a flight or a page away.
#[gpui_kit::test]
fn a_paste_arrives_in_front_of_the_camera(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    press(handle, &["cmd-a", "cmd-x"], cx);
    press(handle, &["cmd-v"], cx);

    let (pasted, visible) = cx.update(|cx| {
        let view = workspace.read(cx);
        let bounds = view
            .document(cx)
            .read(cx)
            .content_bounds()
            .expect("the paste landed");
        (bounds.center(), view.camera(cx).visible_world_rect(WINDOW))
    });

    // Centred, not merely on screen: the fixture's own viewport already frames its nodes, so
    // "visible" would pass with no translation at all.
    let centre = visible.center();
    assert!(
        pasted.distance_to(centre) < 1.0,
        "the paste landed at {pasted:?}, not at the centre of the view {centre:?}"
    );
}

/// `cmd-c` over a focused result table copies its cells as TSV — and must not *also* copy the
/// node. Action listeners run leaf to root and the first one ends the dispatch unless it calls
/// `cx.propagate()`, so the table's handler wins on depth. If it did not, the node clipboard
/// would fill behind it and the paste below would put a second result node on the page.
#[gpui_kit::test]
fn copying_in_a_focused_result_does_not_also_copy_the_node(cx: &mut TestAppContext) {
    let (handle, workspace) = open_result(cx);
    let bounds = table_bounds(cx, handle);
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    click_at(&mut visual, cell_at(bounds, 0.0, 0.0));

    press(handle, &["cmd-c", "cmd-v"], cx);

    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(
        copied.as_deref(),
        Some("0"),
        "the table copied its own cell"
    );
    assert_eq!(
        node_ids(cx, &workspace).len(),
        1,
        "and nothing was pasted, so the canvas never copied the node"
    );

    // The control for the assertion above: the same two keys away from the table do paste a
    // node, so "nothing was pasted" means the canvas handler stayed out of it — not that this
    // fixture cannot paste at all.
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    click_at(&mut visual, point(px(1100.0), px(700.0)));
    press(handle, &["cmd-a", "cmd-c", "cmd-v"], cx);
    assert_eq!(node_ids(cx, &workspace).len(), 2);
}
