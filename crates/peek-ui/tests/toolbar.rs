//! The bottom-centre tool palette and the zoom cluster beside it: arming, the active state, that a
//! tool with no registered command is inert rather than silently broken, and that the chrome's
//! buttons dispatch the same actions the keyboard does.

use gpui_kit::KeyContext;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_document::CanvasDocument;
use peek_ui::WorkspaceView;
use peek_ui::commands;

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

#[gpui_kit::test]
fn the_text_tool_arms_place_mode_and_a_click_places_a_node(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = node_count(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("Tool::Text", cx);
        window.render_frame(cx);
    })
    .unwrap();

    // Arming is only meaningful if the next click on the canvas places something.
    cx.update_window(handle.into(), |_, window, cx| {
        let at = gpui_kit::point(px(600.0), px(500.0));
        window.drag(at, at, cx);
    })
    .unwrap();

    assert_eq!(node_count(cx, &workspace), before + 1);
    let placed = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.selected().iter().next().cloned()
    });
    assert!(
        placed.is_some_and(|id| id.as_str().starts_with("text_")),
        "the toolbar armed the text tool, not some other kind"
    );
}

#[gpui_kit::test]
fn the_select_tool_disarms_a_place_tool(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = node_count(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("Tool::Variable", cx);
        window.render_frame(cx);
        window.click("Tool::Select", cx);
        window.render_frame(cx);
        // With nothing armed this is an ordinary canvas click, not a placement.
        let at = gpui_kit::point(px(600.0), px(500.0));
        window.drag(at, at, cx);
    })
    .unwrap();

    assert_eq!(node_count(cx, &workspace), before);
}

#[gpui_kit::test]
fn a_tool_with_no_registered_command_is_present_but_inert(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = node_count(cx, &workspace);

    // `Tool::Agent` has no `COMMANDS` entry until peek-acp lands, so the button renders
    // disabled rather than arming a tool whose node cannot do anything yet.
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(
            window.try_find("Tool::Agent").is_some(),
            "the slot is shown"
        );
        window.click("Tool::Agent", cx);
        window.render_frame(cx);
        let at = gpui_kit::point(px(600.0), px(500.0));
        window.drag(at, at, cx);
    })
    .unwrap();

    assert_eq!(
        node_count(cx, &workspace),
        before,
        "a disabled tool must not arm place mode"
    );
}

/// The toolbar derives itself from the command registry, so a tool becomes usable the moment
/// its command is registered — no edit to the toolbar. `Tool::Query` arrived that way.
#[gpui_kit::test]
fn a_tool_becomes_live_when_its_command_is_registered(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = node_count(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("Tool::Query", cx);
        window.render_frame(cx);
        let at = gpui_kit::point(px(600.0), px(500.0));
        window.drag(at, at, cx);
    })
    .unwrap();

    assert_eq!(
        node_count(cx, &workspace),
        before + 1,
        "Tool::Query is in COMMANDS, so its button arms place mode"
    );
    let placed = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.selected().iter().next().cloned()
    });
    assert!(placed.is_some_and(|id| id.as_str().starts_with("query_")));
}

#[gpui_kit::test]
fn hiding_the_interface_hides_the_toolbar_and_the_title_bar(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("toolbar").is_some());
        assert!(window.try_find("page-tabs").is_some());
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("cmd-.", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("toolbar").is_none(),
            "the toolbar hides too"
        );
        assert!(window.try_find("page-tabs").is_none());
        assert!(window.try_find("canvas").is_some(), "the canvas stays");
    })
    .unwrap();
}

/// The lock is icon-only, so its id is the only thing a test can hold on to — and the reason the
/// HUD's buttons carry ids at all.
#[gpui_kit::test]
fn the_lock_button_toggles_the_camera_lock(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    assert!(!cx.update(|cx| workspace.read(cx).is_camera_locked(cx)));

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("camera-lock", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert!(cx.update(|cx| workspace.read(cx).is_camera_locked(cx)));

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("camera-lock", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert!(
        !cx.update(|cx| workspace.read(cx).is_camera_locked(cx)),
        "the button toggles rather than only locking"
    );
}

/// Locking the camera freezes pan and zoom, so the controls that would change it go with it —
/// otherwise the cluster offers buttons that quietly do nothing visible. Asserted by behaviour
/// rather than by the accessibility flag, which `Button` does not publish, and the same way
/// `a_tool_with_no_registered_command_is_present_but_inert` checks the palette.
#[gpui_kit::test]
fn locking_the_camera_makes_the_zoom_controls_inert(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("camera-lock", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert!(cx.update(|cx| workspace.read(cx).is_camera_locked(cx)));

    let locked_at = cx.update(|cx| workspace.read(cx).camera_target(cx).zoom);
    cx.update_window(handle.into(), |_, window, cx| {
        for id in ["zoom-in", "zoom-out", "zoom-fit", "zoom-reset"] {
            window.click(id, cx);
            window.render_frame(cx);
        }
    })
    .unwrap();

    // Bit-exact on purpose: a locked camera drops the effect entirely, so the zoom is the
    // very same value rather than an arithmetically equal one.
    let after = cx.update(|cx| workspace.read(cx).camera_target(cx).zoom);
    assert!(
        (after - locked_at).abs() < f64::EPSILON,
        "a frozen camera must not move for any of the zoom buttons: {locked_at} -> {after}"
    );

    // And the lock is still the way out.
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("camera-lock", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert!(!cx.update(|cx| workspace.read(cx).is_camera_locked(cx)));
}

#[gpui_kit::test]
fn the_zoom_buttons_dispatch_through_the_canvas(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = cx.update(|cx| workspace.read(cx).camera_target(cx).zoom);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("zoom-in", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let after = cx.update(|cx| workspace.read(cx).camera_target(cx).zoom);
    assert!(
        after > before,
        "zoom-in stepped the camera: {before} -> {after}"
    );
}

/// Every chrome button shows its shortcut in its tooltip, and gpui resolves that shortcut by
/// parsing the context string the button was given. `KeyContext::parse` reads bare identifiers:
/// on a binding predicate's `&` it consumes nothing and recurses on the same input until the
/// stack is gone, so handing one to a tooltip aborts the process the moment a button is hovered.
/// Hovering is the only way to reach that code, which is why clicking through the toolbar in the
/// other tests never caught it.
#[gpui_kit::test]
fn hovering_a_chrome_button_renders_its_shortcut_tooltip(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    for id in ["Tool::Text", "zoom-in", "camera-lock", "page-add"] {
        cx.update_window(handle.into(), |_, window, cx| window.hover(id, cx))
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
}

/// The other half of the same bug: a context that parses but matches nothing would trade the
/// crash for a tooltip that silently drops its shortcut. These are the two contexts the chrome
/// hands to `tooltip_with_action`, and both have to find the binding the keyboard would use.
#[gpui_kit::test]
fn a_chrome_tooltip_context_resolves_the_binding_the_keyboard_uses(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    let cases: [(&str, &dyn gpui_kit::Action); 2] = [
        (commands::CANVAS, &commands::actions::zoom::In),
        (
            commands::WORKSPACE,
            &commands::actions::command_palette::Open,
        ),
    ];
    cx.update_window(handle.into(), |_, window, _| {
        for (context, action) in cases {
            let context = KeyContext::parse(context).expect("a bare context, not a predicate");
            assert!(
                window
                    .highest_precedence_binding_for_action_in_context(action, context)
                    .is_some(),
                "{} has no binding in the context its tooltip names",
                action.name()
            );
        }
    })
    .unwrap();
}
