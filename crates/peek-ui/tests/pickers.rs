//! Pages, the two title-bar pickers and the page-display setting, driven through real key and
//! pointer dispatch.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_config::PageDisplay;
use peek_document::geometry::{Point as World, Rect, Size as WorldSize};
use peek_document::{CanvasDocument, NodeId, NodeType, PageId};
use peek_ui::WorkspaceView;
use peek_ui::commands::actions;

const FIXTURE: &str = include_str!("../../peek-document/tests/fixtures/plock-local.json");

fn open_as(
    cx: &mut TestAppContext,
    display: PageDisplay,
) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        config.ui.pages.show_as = display;
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

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    open_as(cx, PageDisplay::Tabs)
}

fn render(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
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

fn selection(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<NodeId> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).selected().iter().cloned().collect()
    })
}

/// Adds three queries whose x order is deliberately not their document order, then reports
/// *every* query on the page left to right — the fixture brings its own, and the commands cycle
/// through all of them.
fn queries_left_to_right(
    cx: &mut TestAppContext,
    workspace: &Entity<WorkspaceView>,
) -> Vec<NodeId> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            for x in [-9000.0, -9400.0, -9200.0] {
                document.create_node(
                    NodeType::Query,
                    Rect::new(World::new(x, 0.0), WorldSize::new(100.0, 100.0)),
                );
            }
            document.select_only([]);
            cx.notify();

            let mut queries: Vec<(f64, NodeId)> = document
                .nodes()
                .iter()
                .filter(|node| node.node_type() == Some(NodeType::Query))
                .map(|node| (node.position.x, node.id.clone()))
                .collect();
            queries.sort_by(|left, right| left.0.total_cmp(&right.0));
            queries.into_iter().map(|(_, id)| id).collect()
        })
    })
}

/// The palette's only runtime rows. Every page but the one you are on, and no other "Go to"
/// row leaks in from the registry.
#[gpui_kit::test]
fn the_palette_offers_one_go_to_row_per_inactive_page(cx: &mut TestAppContext) {
    let (_, workspace) = open(cx);
    let ids = pages(cx, &workspace);
    assert!(ids.len() > 1, "the fixture has more than one page");

    // By action name, not by title: `Page::GoToNode` is also called "Go to a node".
    let rows: Vec<String> = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        peek_ui::commands::palette::entries(document.read(cx), &peek_canvas::Scope::default())
            .iter()
            .filter(|entry| entry.action.name() == "Page::GoTo")
            .map(|entry| entry.title.to_string())
            .collect()
    });

    assert_eq!(rows.len(), ids.len() - 1, "{rows:?}");
    let active_name = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).active_page().name.clone()
    });
    assert!(
        !rows.contains(&format!("Go to {active_name}")),
        "the active page is not somewhere to go: {rows:?}"
    );
}

/// `Page::GoTo` carries its target, has no binding, and is dispatched by the palette through the
/// canvas focus handle — the path that silently swallowed node-scoped commands before.
#[gpui_kit::test]
fn going_to_a_page_switches_to_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let start = active(cx, &workspace);
    let target = pages(cx, &workspace)
        .into_iter()
        .find(|id| id != &start)
        .expect("a second page");

    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_action(
            Box::new(actions::page::GoTo {
                page: target.clone(),
            }),
            cx,
        );
    })
    .unwrap();

    assert_eq!(active(cx, &workspace), target);
}

/// `⌘]` walks the page's queries left to right and wraps; `⌘[` walks back. Nothing selected
/// anchors before the first, so the first press lands on either end.
#[gpui_kit::test]
fn the_bracket_keys_cycle_the_pages_queries(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let ordered = queries_left_to_right(cx, &workspace);
    render(cx, handle);

    for expected in &ordered {
        cx.update_window(handle.into(), |_, window, cx| window.press("cmd-]", cx))
            .unwrap();
        assert_eq!(selection(cx, &workspace), vec![expected.clone()]);
    }
    // Wrapped back to the leftmost.
    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-]", cx))
        .unwrap();
    assert_eq!(selection(cx, &workspace), vec![ordered[0].clone()]);

    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-[", cx))
        .unwrap();
    assert_eq!(
        selection(cx, &workspace),
        vec![ordered[ordered.len() - 1].clone()],
        "previous from the leftmost wraps to the rightmost"
    );
}

/// The setting decides which of the two shapes the title bar takes, and the command flips it.
#[gpui_kit::test]
fn toggling_the_page_display_swaps_the_tab_strip_for_a_pill(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("page-tabs").is_some(), "tabs by default");
        assert!(window.try_find("page-picker").is_none());
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_action(Box::new(actions::settings::TogglePageDisplay), cx);
    })
    .unwrap();
    render(cx, handle);

    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("page-picker").is_some(), "now a pill");
        assert!(window.try_find("page-tabs").is_none());
    })
    .unwrap();
}

/// The palette row names what pressing it does, not the mode that is up, so it has to invert
/// with `scope.settings.pages_as_list` — the reference's `togglePageDisplay.tsx` wording.
///
/// This exercises the label function, not `CanvasView::scope`'s projection of the setting into
/// it: that method is `pub(crate)`, so an integration test cannot reach the real scope.
#[gpui_kit::test]
fn the_page_display_row_names_the_mode_it_switches_to(cx: &mut TestAppContext) {
    let _ = open(cx);
    let command = peek_ui::commands::find("Settings::TogglePageDisplay").expect("it is registered");

    let tabs = peek_canvas::Scope::default();
    assert_eq!(command.label(&tabs), "Show pages as list");

    let list = peek_canvas::Scope {
        settings: peek_canvas::SettingsScope {
            pages_as_list: true,
            ..peek_canvas::SettingsScope::default()
        },
        ..peek_canvas::Scope::default()
    };
    assert_eq!(command.label(&list), "Show pages as tabs");
}

/// `o` puts the list up in list mode. It stays bare — no modifier — so the binding also has to
/// survive the display switch.
///
/// The tabs-mode half only documents intent: in tabs mode the list has no trigger to hang from,
/// so it would be absent whether or not `open_pages_picker` checked the setting, and
/// `toggle_page_display` closes the picker on the way past. The guard itself is what keeps the
/// open flag from being left set, and nothing outside this crate can observe that.
#[gpui_kit::test]
fn o_opens_the_pages_picker_in_list_mode(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| window.press("o", cx))
        .unwrap();
    render(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(
            window.try_find("pages-list").is_none(),
            "nothing in tabs mode"
        );
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_action(Box::new(actions::settings::TogglePageDisplay), cx);
    })
    .unwrap();
    render(cx, handle);

    cx.update_window(handle.into(), |_, window, cx| window.press("o", cx))
        .unwrap();
    cx.run_until_parked();
    render(cx, handle);

    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("pages-list").is_some(), "the list is up");
    })
    .unwrap();
}

/// Choosing a row switches to that page and puts the list away.
#[gpui_kit::test]
fn the_pages_picker_switches_pages(cx: &mut TestAppContext) {
    let (handle, workspace) = open_as(cx, PageDisplay::List);
    let start = active(cx, &workspace);
    let target = pages(cx, &workspace)
        .into_iter()
        .find(|id| id != &start)
        .expect("a second page");

    cx.update_window(handle.into(), |_, window, cx| window.press("o", cx))
        .unwrap();
    cx.run_until_parked();
    render(cx, handle);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(format!("page-row-{target}"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    render(cx, handle);

    assert_eq!(active(cx, &workspace), target);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(
            window.try_find("pages-list").is_none(),
            "and closes behind it"
        );
    })
    .unwrap();
}
