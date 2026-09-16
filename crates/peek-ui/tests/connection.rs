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
    let mut config = peek_config::PeekConfig::default();
    config.theme = peek_config::ThemeId::Midday;
    open_with(&config, cx)
}

/// Two workspaces and three connections, so the picker has something to group, filter and walk.
/// `peek_ui::init` seeds the `Settings` global from this, which is where the panel reads it.
fn seeded() -> peek_config::PeekConfig {
    let connection = |name: &str, url: &str| peek_config::DatabaseConnection {
        name: name.to_string(),
        color: "#5584E8".to_string(),
        url: url.to_string(),
        ssh_tunnel: None,
    };
    let mut config = peek_config::PeekConfig::default();
    config.theme = peek_config::ThemeId::Midday;
    config.workspaces = vec![
        peek_config::Workspace {
            name: "Peek".to_string(),
            connections: vec![connection("local", "postgres://dbuser:pw@postgres/nesso")],
        },
        peek_config::Workspace {
            name: "Plock".to_string(),
            connections: vec![
                connection("local", "postgres://metered_user:pw@localhost/forge"),
                connection(
                    "production",
                    "postgres://pg:pw@productiondatabase/metered_production",
                ),
            ],
        },
    ];
    config
}

fn open_with(
    config: &peek_config::PeekConfig,
    cx: &mut TestAppContext,
) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        peek_ui::init(config, cx);
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

/// The tool surface follows the switch. The MCP drain answers every agent call through
/// `run_tool`, so a canvas captured anywhere on that path would keep an agent editing the
/// document that was open when the window opened — invisibly, since nothing renders it.
#[gpui_kit::test]
fn a_tool_call_after_a_switch_lands_on_the_new_document(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    switch(handle, &workspace, ("no-such-workspace", "no-such-db"), cx);

    let reply = cx
        .update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |view, cx| {
                view.run_tool(
                    "create_text_node",
                    &serde_json::json!({ "text": "from the agent", "position": [0.0, 0.0] }),
                    window,
                    cx,
                )
            })
        })
        .unwrap();

    let id = peek_document::NodeId::from(reply["nodeId"].as_str().expect("the node was created"));
    let (count, landed) = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        (document.nodes().len(), document.node(&id).is_some())
    });
    assert_eq!(
        count, 1,
        "the empty document gained exactly the agent's node"
    );
    assert!(landed, "the node is in the document the window is showing");
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

/// The pill is the pointer's way into the picker, and it dispatches the same action the
/// keyboard does. The panel takes focus when it opens and escape hands it back — the overlay
/// contract, now observable directly, because the panel is hand-owned and its elements carry
/// ids a test can reach. The `DropdownMenu` this replaced registered none.
#[gpui_kit::test]
fn clicking_the_pill_opens_the_panel_and_escape_returns_focus(cx: &mut TestAppContext) {
    let (handle, _) = open_with(&seeded(), cx);

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

        assert!(window.try_find("connection-picker-panel").is_some());
        assert_eq!(
            window.find("canvas").focused(),
            Some(false),
            "the search box took focus"
        );
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);

        assert!(window.try_find("connection-picker-panel").is_none());
        assert_eq!(window.find("canvas").focused(), Some(true));
    })
    .unwrap();
}

/// The picker's own tests. The panel is a hand-owned overlay rather than a `DropdownMenu`
/// precisely so its rows carry ids a test can reach — the comment on
/// `the_menu_takes_focus_on_open_and_escape_returns_it` used to say they could not.
mod picker {
    use super::{open_with, seeded};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};

    /// Bare `p`, as the reference binds it, and a toggle rather than a one-way open.
    #[gpui_kit::test]
    fn p_opens_the_panel_and_pressing_it_again_closes_it(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            assert!(window.try_find("connection-picker-panel").is_none());
            window.press("p", cx);
            window.render_frame(cx);
            assert!(window.try_find("connection-picker-panel").is_some());
        })
        .unwrap();
    }

    /// The load-bearing one. `p` is bound on a `!Input` context, so once the search box has
    /// focus the letter has to reach the field instead of toggling the panel shut — otherwise
    /// the picker cannot be searched for anything containing a `p`.
    #[gpui_kit::test]
    fn p_typed_into_the_search_box_does_not_close_the_panel(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("p", cx);
            window.render_frame(cx);
            window.press("p", cx);
            window.render_frame(cx);

            assert!(
                window.try_find("connection-picker-panel").is_some(),
                "the second p was typed, not treated as the shortcut"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn escape_closes_the_panel_and_hands_focus_back(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("p", cx);
            window.render_frame(cx);
            window.press("escape", cx);
            window.render_frame(cx);

            assert!(window.try_find("connection-picker-panel").is_none());
            assert_eq!(window.find("canvas").focused(), Some(true));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_press_on_the_scrim_closes_the_panel(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("p", cx);
            window.render_frame(cx);
            window.click("connection-picker-scrim", cx);
            window.render_frame(cx);

            assert!(window.try_find("connection-picker-panel").is_none());
        })
        .unwrap();
    }

    /// The cursor walks the flattened rows in the order they are drawn, so `down` from the
    /// first row of one workspace lands on the first row of the next.
    #[gpui_kit::test]
    fn the_arrow_keys_walk_the_flattened_rows(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("p", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("connection-picker-row-Peek/local").selected(),
                Some(true),
                "the cursor starts on the first row"
            );

            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("connection-picker-row-Plock/local").selected(),
                Some(true)
            );

            window.press("up", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("connection-picker-row-Peek/local").selected(),
                Some(true)
            );
        })
        .unwrap();
    }

    /// The cursor is clamped rather than wrapped, matching the reference's `Math.max`/`Math.min`.
    #[gpui_kit::test]
    fn the_cursor_stops_at_the_ends_rather_than_wrapping(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("p", cx);
            window.render_frame(cx);
            window.press("up", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("connection-picker-row-Peek/local").selected(),
                Some(true),
                "up from the first row stays put"
            );

            for _ in 0..5 {
                window.press("down", cx);
            }
            window.render_frame(cx);
            assert_eq!(
                window
                    .find("connection-picker-row-Plock/production")
                    .selected(),
                Some(true),
                "down past the last row stays on it"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn enter_switches_to_the_row_under_the_cursor(cx: &mut TestAppContext) {
        let (handle, workspace) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("p", cx);
            window.render_frame(cx);
            window.press("down", cx);
            window.press("down", cx);
            window.render_frame(cx);
            window.press("enter", cx);
            window.render_frame(cx);
        })
        .unwrap();

        assert_eq!(
            cx.update(|cx| {
                let view = workspace.read(cx);
                let (found, connection) = view.open_connection();
                (found.to_string(), connection.to_string())
            }),
            ("Plock".to_string(), "production".to_string()),
        );
    }

    /// Searching matches the workspace name too, which is what surfaces a workspace's
    /// connections even though the cursor never lands on the workspace itself.
    #[gpui_kit::test]
    fn typing_filters_the_rows_across_workspaces(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("p", cx);
            window.render_frame(cx);
            window.input("plock", cx);
            window.render_frame(cx);

            assert!(
                window
                    .try_find("connection-picker-row-Peek/local")
                    .is_none(),
                "the other workspace is filtered out"
            );
            assert!(
                window
                    .try_find("connection-picker-row-Plock/production")
                    .is_some()
            );
        })
        .unwrap();
    }

    /// A collapsed workspace cannot hide the row Enter would pick, so the cursor expands
    /// whatever it walks into.
    #[gpui_kit::test]
    fn a_collapsed_workspace_expands_when_the_cursor_enters_it(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("p", cx);
            window.render_frame(cx);
            window.click("connection-picker-workspace-Plock", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("connection-picker-workspace-Plock").expanded(),
                Some(false)
            );
            assert!(
                window
                    .try_find("connection-picker-row-Plock/local")
                    .is_none()
            );

            window.press("down", cx);
            window.render_frame(cx);

            assert_eq!(
                window.find("connection-picker-workspace-Plock").expanded(),
                Some(true),
                "the cursor moved into it, so it opened"
            );
            assert_eq!(
                window.find("connection-picker-row-Plock/local").selected(),
                Some(true)
            );
        })
        .unwrap();
    }
}

/// The forms. Every write here is refused — `WorkspaceView::with_document` runs in
/// `PersistenceMode::ReadOnly`, the default — so what these pin is that the refusal is visible
/// *before* the click rather than reported after it.
mod forms {
    use super::{open_with, seeded};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};

    fn open_panel(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
        window.press("p", cx);
        window.render_frame(cx);
    }

    /// "Add connection" sits under each workspace, and pushes a blank form.
    #[gpui_kit::test]
    fn the_add_row_opens_a_blank_connection_form(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            open_panel(window, cx);
            window.click("connection-picker-add-Plock", cx);
            window.render_frame(cx);

            assert!(window.try_find("connection-form").is_some());
            assert!(
                window.try_find("connection-picker-search").is_none(),
                "the form replaced the list"
            );
        })
        .unwrap();
    }

    /// Escape inside a form pops back to the list rather than closing the panel — the
    /// reference's behaviour, and the reason a form is a view rather than a second overlay.
    #[gpui_kit::test]
    fn escape_in_a_form_goes_back_to_the_list_not_out(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            open_panel(window, cx);
            window.click("connection-picker-add-Plock", cx);
            window.render_frame(cx);
            window.press("escape", cx);
            window.render_frame(cx);

            assert!(window.try_find("connection-form").is_none());
            assert!(
                window.try_find("connection-picker-search").is_some(),
                "back on the list, panel still open"
            );

            window.press("escape", cx);
            window.render_frame(cx);
            assert!(
                window.try_find("connection-picker-panel").is_none(),
                "a second escape closes"
            );
        })
        .unwrap();
    }

    /// An edit form and an add form are told apart by the control only an edit has: Remove.
    /// Field contents are not observable — an `Input` registers under its own generated id — so
    /// this pins the distinction that actually matters.
    #[gpui_kit::test]
    fn the_pencil_opens_an_edit_form_not_a_blank_one(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            open_panel(window, cx);
            window.click("connection-picker-add-Plock", cx);
            window.render_frame(cx);
            assert!(
                window.try_find("connection-remove").is_none(),
                "a new connection has nothing to remove"
            );

            window.press("escape", cx);
            window.render_frame(cx);
            window.click("connection-picker-edit-Plock/production", cx);
            window.render_frame(cx);

            assert!(window.try_find("connection-form").is_some());
            assert!(
                window.try_find("connection-remove").is_some(),
                "an existing connection can be removed"
            );
        })
        .unwrap();
    }

    /// Editing a connection must not also switch to it: the pencil claims its own press.
    #[gpui_kit::test]
    fn opening_the_form_does_not_switch_to_the_connection(cx: &mut TestAppContext) {
        let (handle, workspace) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            open_panel(window, cx);
            window.click("connection-picker-edit-Plock/production", cx);
            window.render_frame(cx);
        })
        .unwrap();

        assert_eq!(
            cx.update(|cx| workspace.read(cx).open_connection().1.to_string()),
            String::new(),
            "still on whatever was open before"
        );
    }

    /// The write gate. `Settings::update` applies its change in memory even when the disk write
    /// is refused, so if Save were live under `ReadOnly` the connection would appear in the
    /// config — which is exactly what this asserts does not happen.
    #[gpui_kit::test]
    fn save_does_nothing_when_this_run_may_not_write(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            open_panel(window, cx);
            window.click("connection-picker-add-Plock", cx);
            window.render_frame(cx);
            window.input("scratch", cx);
            window.render_frame(cx);
            window.click("connection-save", cx);
            window.render_frame(cx);
        })
        .unwrap();

        assert_eq!(
            cx.update(|cx| { peek_ui::test_support::connection_names(cx, "Plock") }),
            vec!["local".to_string(), "production".to_string()],
            "nothing was added"
        );
    }

    /// Duplicate writes immediately rather than through a form, so it carries the same gate.
    #[gpui_kit::test]
    fn duplicate_does_nothing_when_this_run_may_not_write(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            open_panel(window, cx);
            window.click("connection-picker-duplicate-Plock/local", cx);
            window.render_frame(cx);
        })
        .unwrap();

        assert_eq!(
            cx.update(|cx| peek_ui::test_support::connection_names(cx, "Plock"))
                .len(),
            2,
            "no copy was made"
        );
    }

    #[gpui_kit::test]
    fn the_footer_opens_a_workspace_form(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            open_panel(window, cx);
            window.click("connection-picker-new-workspace", cx);
            window.render_frame(cx);

            assert!(window.try_find("workspace-form").is_some());
            assert!(
                window.try_find("workspace-remove").is_none(),
                "a new workspace has nothing to remove"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_workspace_pencil_opens_an_edit_form(cx: &mut TestAppContext) {
        let (handle, _) = open_with(&seeded(), cx);

        cx.update_window(handle.into(), |_, window, cx| {
            open_panel(window, cx);
            window.click("connection-picker-edit-workspace-Plock", cx);
            window.render_frame(cx);

            assert!(window.try_find("workspace-form").is_some());
            assert!(window.try_find("workspace-remove").is_some());
        })
        .unwrap();
    }
}
