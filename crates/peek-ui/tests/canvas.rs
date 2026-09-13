//! Milestone 1 acceptance, driven through real gpui event dispatch in a headless window.

use std::time::Duration;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, PinchEvent, Pixels, Point, ScrollDelta, TestAppContext,
    TouchPhase, VisualTestContext, WindowHandle, point, px, size,
};
use peek_canvas::{Camera, MAX_ZOOM};
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
    (handle, workspace.unwrap())
}

fn settle(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    // Flights run on wall-clock time (300 ms at most); render once more after they finish.
    std::thread::sleep(Duration::from_millis(350));
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// `focusCreated` in `executeQueries.ts`: a finished run selects the nodes it placed and flies
/// the camera to them, so a result that landed off-screen is what the user is looking at.
#[gpui_kit::test]
fn a_finished_run_frames_the_node_it_placed(cx: &mut TestAppContext) {
    use peek_document::NodeType;
    use peek_document::geometry::{Point as World, Rect, Size as WorldSize};

    let (handle, workspace) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    let initial = cx.update(|cx| workspace.read(cx).camera(cx));

    // Far enough from the fixture that no starting camera could already be framing it.
    let placed = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            let id = document.create_node(
                NodeType::Result,
                Rect::new(World::new(9000.0, 9000.0), WorldSize::new(600.0, 440.0)),
            );
            document.focus_created(vec![id.clone()]);
            cx.notify();
            id
        })
    });

    let target = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!(!target.approx_eq(initial), "the run moves the camera");
    assert!(
        target.zoom <= 1.0,
        "framing never zooms past 100%: {}",
        target.zoom
    );

    cx.update(|cx| {
        let view = workspace.read(cx);
        let document = view.document(cx);
        let document = document.read(cx);
        assert_eq!(
            document.selected().iter().collect::<Vec<_>>(),
            vec![&placed],
            "and selects what it placed"
        );
        let bounds = document.bounds_of([&placed]).unwrap();
        let visible = target.visible_world_rect(peek_canvas::Size::new(1200.0, 800.0));
        assert!(
            visible.contains(bounds.min()) && visible.contains(bounds.max()),
            "the placed node is in frame: {bounds:?} in {visible:?}"
        );
    });

    settle(cx, handle);
}

/// A second run that only refreshed the result already on screen created nothing, so it leaves
/// the camera where the user left it.
#[gpui_kit::test]
fn a_run_that_creates_nothing_leaves_the_camera_alone(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    let initial = cx.update(|cx| workspace.read(cx).camera(cx));

    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            document.focus_created(Vec::new());
            cx.notify();
        });
    });

    let target = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!(target.approx_eq(initial));
}

#[gpui_kit::test]
fn fit_view_frames_all_nodes_and_commits_the_viewport(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let initial = cx.update(|cx| workspace.read(cx).camera(cx));

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // What macOS delivers for cmd-shift-0: shifted non-letters arrive as the character
        // they type, with the shift flag cleared (`peek_config::keymap::shifted`).
        window.press("cmd-)", cx);
    })
    .unwrap();

    let target = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!(!target.approx_eq(initial), "fit view must move the camera");
    assert!(
        target.zoom <= 1.0,
        "fit view never zooms past 100%: {}",
        target.zoom
    );

    settle(cx, handle);
    cx.update(|cx| {
        let view = workspace.read(cx);
        assert!(
            view.camera(cx).approx_eq(target),
            "flight lands on its target"
        );
        let document = view.document(cx);
        let viewport = document.read(cx).viewport();
        assert!(
            Camera::from_viewport(viewport).approx_eq(target),
            "viewport is committed when the flight ends"
        );
        let content = document.read(cx).content_bounds().unwrap();
        let visible = target.visible_world_rect(peek_canvas::Size::new(1200.0, 800.0));
        assert!(visible.contains(content.min()) && visible.contains(content.max()));
    });
}

#[gpui_kit::test]
fn wheel_pans_and_camera_lock_freezes_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = cx.update(|cx| workspace.read(cx).camera(cx));

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.scroll("canvas", ScrollDelta::Pixels(point(px(30.0), px(50.0))), cx);
    })
    .unwrap();
    let panned = cx.update(|cx| workspace.read(cx).camera(cx));
    assert!(
        (panned.pan.x - (before.pan.x + 30.0)).abs() < 0.01,
        "{panned:?}"
    );
    assert!(
        (panned.pan.y - (before.pan.y + 50.0)).abs() < 0.01,
        "{panned:?}"
    );
    assert!(
        (panned.zoom - before.zoom).abs() < 1e-9,
        "plain wheel never zooms"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("cmd-shift-l", cx);
        window.scroll("canvas", ScrollDelta::Pixels(point(px(0.0), px(80.0))), cx);
    })
    .unwrap();
    cx.update(|cx| {
        let view = workspace.read(cx);
        assert!(view.is_camera_locked(cx));
        assert!(
            view.camera(cx).approx_eq(panned),
            "locked camera ignores the wheel"
        );
    });
}

#[gpui_kit::test]
fn pinch_zooms_about_the_pointer_and_clamps(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    let before = cx.update(|cx| workspace.read(cx).camera(cx));

    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    let anchor: Point<Pixels> = point(px(600.0), px(500.0));
    visual.simulate_event(PinchEvent {
        position: anchor,
        delta: 0.25,
        modifiers: Modifiers::default(),
        phase: TouchPhase::Moved,
    });
    let zoomed = cx.update(|cx| workspace.read(cx).camera(cx));
    assert!(
        (zoomed.zoom / before.zoom - 1.25).abs() < 1e-6,
        "{zoomed:?}"
    );
    assert!(
        (zoomed.pan - before.pan).length() > 0.0,
        "zooming about an off-centre anchor shifts the pan"
    );

    for _ in 0..40 {
        visual.simulate_event(PinchEvent {
            position: anchor,
            delta: 1.0,
            modifiers: Modifiers::default(),
            phase: TouchPhase::Moved,
        });
    }
    let clamped = cx.update(|cx| workspace.read(cx).camera(cx));
    assert!((clamped.zoom - MAX_ZOOM).abs() < 1e-9, "{clamped:?}");
}

#[gpui_kit::test]
fn escape_and_select_all_reach_the_canvas(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-a", cx);
    })
    .unwrap();
    let document = cx.update(|cx| workspace.read(cx).document(cx));
    let selected = cx.update(|cx| document.read(cx).selected().len());
    let total = cx.update(|cx| document.read(cx).nodes().len());
    assert_eq!(selected, total);
    assert!(total > 0);

    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    assert_eq!(cx.update(|cx| document.read(cx).selected().len()), 0);
}

#[gpui_kit::test]
fn palette_open_and_dismiss_keeps_canvas_commands_working(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let focused_before = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let focused = window.find("canvas").focused();
            window.press("cmd-p", cx);
            focused
        })
        .unwrap();
    cx.run_until_parked();
    let focused_open = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let focused = window.find("canvas").focused();
            window.press("escape", cx);
            focused
        })
        .unwrap();
    cx.run_until_parked();
    let focused_after = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let focused = window.find("canvas").focused();
            window.press("cmd-0", cx);
            focused
        })
        .unwrap();
    let target = cx.update(|cx| workspace.read(cx).camera_target(cx));
    assert!(
        (target.zoom - 1.0).abs() < 1e-9,
        "reset after the palette closes: focus before {focused_before:?} open {focused_open:?} after {focused_after:?}: {target:?}"
    );
}

#[gpui_kit::test]
fn page_switching_restores_each_pages_viewport(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let document = cx.update(|cx| workspace.read(cx).document(cx));
    let first_page = cx.update(|cx| document.read(cx).active_page_id().clone());
    let first_camera = cx.update(|cx| workspace.read(cx).camera(cx));

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-}", cx);
    })
    .unwrap();
    let second_page = cx.update(|cx| document.read(cx).active_page_id().clone());
    assert_ne!(first_page, second_page);
    let second_camera = cx.update(|cx| workspace.read(cx).camera(cx));
    let stored = cx.update(|cx| Camera::from_viewport(document.read(cx).viewport()));
    assert!(
        second_camera.approx_eq(stored),
        "camera follows the page's viewport"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("cmd-{", cx);
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| document.read(cx).active_page_id().clone()),
        first_page
    );
    assert!(
        cx.update(|cx| workspace.read(cx).camera(cx))
            .approx_eq(first_camera)
    );
}

#[gpui_kit::test]
fn theme_picker_previews_reverts_and_commits(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme;
    use gpui_kit::component::theme::ThemeMode;
    use peek_config::ThemeId;
    use peek_theme::{ActivePeekTheme, ThemeService};

    let (handle, _workspace) = open(cx);
    cx.update(|cx| {
        assert_eq!(ThemeService::committed(cx), ThemeId::Midday);
        assert_eq!(cx.theme().mode, ThemeMode::Light);
        assert!(cx.peek_theme().is_light);
    });

    let open_picker = |cx: &mut TestAppContext| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(Box::new(peek_ui::commands::actions::theme::Open), cx);
        })
        .unwrap();
        cx.run_until_parked();
    };

    // Browse with the arrow keys: the theme under the cursor is previewed, Escape reverts.
    open_picker(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_ne!(
            ThemeService::effective(cx),
            ThemeId::Midday,
            "arrow previews"
        );
        assert_eq!(ThemeService::committed(cx), ThemeId::Midday);
    });
    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            ThemeService::effective(cx),
            ThemeId::Midday,
            "escape reverts"
        );
        assert!(cx.peek_theme().is_light);
    });

    // Enter commits the previewed theme and the component theme follows its mode.
    open_picker(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("up", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let committed = ThemeService::committed(cx);
        assert_ne!(
            committed,
            ThemeId::Midday,
            "enter commits the previewed theme"
        );
        assert_eq!(ThemeService::effective(cx), committed);
        assert_eq!(cx.theme().mode.is_dark(), !cx.peek_theme().is_light);
    });
}
