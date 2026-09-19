//! Regions and wayfinding, driven through real key and pointer dispatch: grouping, ungrouping,
//! the picker, the beacons a zoomed-out canvas is navigated by, and the settings toggle.
//!
//! Nothing here reads a pixel — the harness installs a no-op text system — so what these assert
//! is the document and the element tree. How the halo actually looks is a feel-test item.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_canvas::Camera;
use peek_document::{CanvasDocument, NodeId, PageId, RegionStatus};
use peek_ui::WorkspaceView;

const FIXTURE: &str = include_str!("../../peek-document/tests/fixtures/plock-local.json");
/// The fixture page that already carries regions, as the real `~/peek` documents do.
const REGION_PAGE: &str = "page_hbnrnVts";
/// "Deal Subscriptions" and its five members.
const DEALS: &str = "region_AYKG_q6o";
/// "User Search", two members — small enough to delete whole.
const SEARCH: &str = "region_eLgfKWTj";
const SEARCH_MEMBERS: [&str; 2] = ["query_Iot82jq3", "query_Iot82jq3-result-0"];
/// "Metered Plans" and its two members. Every node on the page starts in a region, which is
/// what a real grouped document looks like — the tests that need loose nodes make them.
const METERED: &str = "region_N6iqZ1js";
const METERED_MEMBERS: [&str; 2] = ["query_TVz-Cznu", "query_TVz-Cznu-result-0"];

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
    let workspace = workspace.unwrap();
    // Every test works on the page that has regions; the fixture opens on another one.
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("page-tab-{REGION_PAGE}"), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(active_page(cx, &workspace).to_string(), REGION_PAGE);
    (handle, workspace)
}

fn active_page(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> PageId {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).active_page_id().clone()
    })
}

/// Frees two nodes from the region that holds them, so there is something loose to group.
fn loosen(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) {
    let ids: Vec<NodeId> = METERED_MEMBERS.iter().map(|id| NodeId::from(*id)).collect();
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            document.remove_from_regions(&ids);
            document.checkpoint();
            cx.notify();
        });
    });
}

/// Selects through the mutation API, as the docs prescribe for tests: there is no pointer
/// gesture that reaches a node parked off screen at the fixture's zoom.
fn select(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>, ids: &[&str]) {
    let ids: Vec<NodeId> = ids.iter().map(|id| NodeId::from(*id)).collect();
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            document.select_only(ids);
            cx.notify();
        });
    });
}

fn regions(
    cx: &mut TestAppContext,
    workspace: &Entity<WorkspaceView>,
) -> Vec<(String, Vec<String>)> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .regions()
            .iter()
            .map(|region| {
                (
                    region.id.to_string(),
                    region
                        .member_ids
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect(),
                )
            })
            .collect()
    })
}

fn members(
    cx: &mut TestAppContext,
    workspace: &Entity<WorkspaceView>,
    region: &str,
) -> Vec<String> {
    regions(cx, workspace)
        .into_iter()
        .find(|(id, _)| id == region)
        .map(|(_, members)| members)
        .unwrap_or_default()
}

fn node_count(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> usize {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).nodes().len()
    })
}

/// A region fly is 600 ms, so a flight has to be waited out before the camera has arrived.
fn settle(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    std::thread::sleep(std::time::Duration::from_millis(700));
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn camera(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Camera {
    cx.update(|cx| workspace.read(cx).camera(cx))
}

#[gpui_kit::test]
fn grouping_two_nodes_creates_a_region_and_opens_the_picker_to_name_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    loosen(cx, &workspace);
    let before = regions(cx, &workspace).len();
    select(cx, &workspace, &METERED_MEMBERS);

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-g", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let after = regions(cx, &workspace);
    assert_eq!(after.len(), before + 1);
    assert_eq!(
        after.last().unwrap().1,
        METERED_MEMBERS.map(String::from).to_vec(),
        "the new region holds exactly the selection"
    );
    // The naming hand-off: a placeholder name is only acceptable because the field to replace
    // it is already open and selected.
    cx.update_window(handle.into(), |_, window, _| {
        assert!(
            window.try_find("regions-list").is_some(),
            "the picker is up"
        );
        assert!(
            window.try_find("region-rename").is_some(),
            "with the new region in rename mode"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn grouping_a_selection_that_touches_one_region_folds_into_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    loosen(cx, &workspace);
    let before = regions(cx, &workspace).len();
    // One member of "User Search" plus a node no region holds.
    select(cx, &workspace, &[SEARCH_MEMBERS[0], METERED_MEMBERS[0]]);

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-g", cx);
        window.render_frame(cx);
    })
    .unwrap();

    assert_eq!(
        regions(cx, &workspace).len(),
        before,
        "folding grows a region rather than minting one"
    );
    assert!(members(cx, &workspace, SEARCH).contains(&METERED_MEMBERS[0].to_string()));
    // Nothing to name, so the picker stays down — the flash ring is the cue instead.
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("regions-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn re_grouping_a_regions_own_members_does_nothing(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = regions(cx, &workspace);
    select(cx, &workspace, &SEARCH_MEMBERS);

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-g", cx);
        window.render_frame(cx);
    })
    .unwrap();

    assert_eq!(regions(cx, &workspace), before);
}

#[gpui_kit::test]
fn ungrouping_pulls_the_selection_out_and_drops_what_it_empties(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    select(cx, &workspace, &SEARCH_MEMBERS);

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-shift-g", cx);
        window.render_frame(cx);
    })
    .unwrap();

    assert!(
        !regions(cx, &workspace).iter().any(|(id, _)| id == SEARCH),
        "the region lost every member, so it went with them"
    );
    assert_eq!(
        node_count(cx, &workspace),
        9,
        "ungrouping never deletes a node"
    );
}

#[gpui_kit::test]
fn r_opens_the_picker_and_escape_hands_focus_back_to_the_canvas(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("r", cx);
        window.render_frame(cx);
        assert!(window.try_find("regions-list").is_some());

        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("regions-list").is_none());
        assert_eq!(window.find("canvas").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_picker_lists_every_region_and_counts_what_is_ungrouped(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("r", cx);
        window.render_frame(cx);
        for region in [DEALS, SEARCH, METERED] {
            assert!(
                window.try_find(format!("region-row-{region}")).is_some(),
                "{region} has a row"
            );
        }
        // Every node on this page starts in a region, so there is nothing left to report.
        assert!(window.try_find("region-row-ungrouped").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_picker_row_flies_the_camera_to_its_region(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = camera(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("r", cx);
        window.render_frame(cx);
        window.click(format!("region-row-{SEARCH}"), cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, handle);

    let after = camera(cx, &workspace);
    assert!(
        after != before,
        "the camera moved toward the region: {before:?} -> {after:?}"
    );
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("regions-list").is_none(), "and it closed");
    })
    .unwrap();
}

#[gpui_kit::test]
fn removing_a_region_from_the_picker_keeps_its_nodes(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let nodes = node_count(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("r", cx);
        window.render_frame(cx);
        window.click(format!("region-remove-{SEARCH}"), cx);
        window.render_frame(cx);
    })
    .unwrap();

    assert_eq!(regions(cx, &workspace).len(), 2, "one region is gone");
    assert_eq!(node_count(cx, &workspace), nodes, "and every node stayed");
}

#[gpui_kit::test]
fn deleting_a_regions_last_node_takes_the_region_with_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    select(cx, &workspace, &SEARCH_MEMBERS);

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("backspace", cx);
        window.render_frame(cx);
    })
    .unwrap();

    assert!(!regions(cx, &workspace).iter().any(|(id, _)| id == SEARCH));
    // One undo, because the prune runs inside the removal's own transaction.
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("cmd-z", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(members(cx, &workspace, SEARCH).len(), 2);
}

#[gpui_kit::test]
fn a_suggested_region_offers_keep_rename_and_dismiss(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    // What an MCP agent's `group_nodes` writes: `suggested` defaults to true there.
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            document.group_nodes(
                METERED_MEMBERS.iter().map(|id| NodeId::from(*id)).collect(),
                peek_canvas::NewRegion {
                    name: "Metered Plans".to_string(),
                    desc: "what the model thought".to_string(),
                    status: RegionStatus::Suggested,
                },
            );
            cx.notify();
        });
    });

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("suggestion-keep").is_some());
        assert!(window.try_find("suggestion-dismiss").is_some());
        window.click("suggestion-keep", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let confirmed = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .regions()
            .iter()
            .all(|region| region.status == RegionStatus::Confirmed)
    });
    assert!(confirmed, "Keep accepts the proposal");
    cx.update_window(handle.into(), |_, window, _| {
        assert!(
            window.try_find("suggestion-keep").is_none(),
            "and the card goes with it"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn dismissing_a_suggestion_removes_the_region_and_keeps_the_nodes(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let nodes = node_count(cx, &workspace);
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.update(cx, |document, cx| {
            document.group_nodes(
                METERED_MEMBERS.iter().map(|id| NodeId::from(*id)).collect(),
                peek_canvas::NewRegion {
                    name: "Guesswork".to_string(),
                    desc: String::new(),
                    status: RegionStatus::Suggested,
                },
            );
            cx.notify();
        });
    });
    let before = regions(cx, &workspace).len();

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("suggestion-dismiss", cx);
        window.render_frame(cx);
    })
    .unwrap();

    assert_eq!(regions(cx, &workspace).len(), before - 1);
    assert_eq!(node_count(cx, &workspace), nodes);
}

#[gpui_kit::test]
fn turning_regions_off_takes_every_surface_with_it(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("r", cx);
        window.render_frame(cx);
        assert!(window.try_find("regions-list").is_some());
        assert!(window.try_find("regions-menu").is_some(), "and its trigger");
    })
    .unwrap();

    // `Window::dispatch_action` defers to the next effect flush, so it has to be the last thing
    // this closure does — an assertion after it would read the frame before the action ran.
    toggle_regions(cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("regions-menu").is_none());
        assert!(window.try_find("regions-list").is_none());
        assert!(
            window.try_find(format!("beacon-{DEALS}")).is_none(),
            "and the beacons with them"
        );
    })
    .unwrap();

    toggle_regions(cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("regions-menu").is_some(), "and back again");
        assert!(window.try_find(format!("beacon-{DEALS}")).is_some());
    })
    .unwrap();
}

fn toggle_regions(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_action(
            Box::new(peek_ui::commands::actions::settings::ToggleRegions),
            cx,
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_zoomed_out_canvas_shows_a_beacon_per_region(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    // The fixture page is stored at zoom 0.125, well past the beacon fade.
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        for region in [DEALS, SEARCH] {
            assert!(
                window.try_find(format!("beacon-{region}")).is_some(),
                "{region} has a beacon"
            );
        }
    })
    .unwrap();

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("cmd-0", cx);
    })
    .unwrap();
    settle(cx, handle);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(
            window.try_find(format!("beacon-{DEALS}")).is_none(),
            "and at 100% the cards carry the canvas instead"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn clicking_a_beacon_flies_to_its_region(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = camera(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("beacon-{DEALS}"), cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx, handle);

    assert!(
        camera(cx, &workspace) != before,
        "a click on a beacon enters its region"
    );
    assert!(
        members(cx, &workspace, DEALS).len() == 5,
        "and a click is not a drag: nothing moved"
    );
}

/// The only way to move a region: it stores no position of its own, so the beacon drags its
/// members and the box re-derives under them.
#[gpui_kit::test]
fn dragging_a_beacon_moves_every_member_and_undoes_in_one_step(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let before = positions(cx, &workspace);

    let bounds = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find(format!("beacon-{DEALS}")).bounds()
        })
        .unwrap();
    let from = bounds.center();
    let to = gpui_kit::point(from.x + px(120.0), from.y + px(60.0));
    cx.update_window(handle.into(), |_, window, cx| {
        window.drag(from, to, cx);
        window.render_frame(cx);
    })
    .unwrap();

    let after = positions(cx, &workspace);
    let members: Vec<&str> = vec![
        "variable_A1D5kJoU",
        "query_OFDWAUT8",
        "query_OFDWAUT8-result-0",
    ];
    for member in &members {
        let (was, now) = (before[*member], after[*member]);
        assert!(
            (now.0 - was.0) > 100.0 && (now.1 - was.1) > 40.0,
            "{member} moved with the region: {was:?} -> {now:?}"
        );
    }
    assert_eq!(
        before["query_Iot82jq3"], after["query_Iot82jq3"],
        "and a node in another region stayed put"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("cmd-z", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(positions(cx, &workspace), before, "one undo puts them back");
}

fn positions(
    cx: &mut TestAppContext,
    workspace: &Entity<WorkspaceView>,
) -> std::collections::BTreeMap<String, (f64, f64)> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .nodes()
            .iter()
            .map(|node| (node.id.to_string(), (node.position.x, node.position.y)))
            .collect()
    })
}

/// Hiding the chrome takes the picker's trigger with it, so an open panel would be stranded —
/// and the beacons are chrome too.
#[gpui_kit::test]
fn hiding_the_interface_takes_the_picker_and_the_beacons(cx: &mut TestAppContext) {
    let (handle, _) = open(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.press("r", cx);
        window.render_frame(cx);
        assert!(window.try_find("regions-list").is_some());
        window.press("cmd-.", cx);
        window.render_frame(cx);
        assert!(window.try_find("regions-list").is_none());
        assert!(window.try_find(format!("beacon-{DEALS}")).is_none());
    })
    .unwrap();
}
