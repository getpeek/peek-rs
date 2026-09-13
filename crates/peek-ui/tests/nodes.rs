//! Node interaction: the header drags, the body is inert to dragging unless cmd is held,
//! corners resize, and the document commands round-trip through undo. Driven through real
//! gpui event dispatch.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, CursorStyle, Entity, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, SharedString, TestAppContext,
    VisualTestContext, WindowHandle, point, px, size,
};
use peek_document::geometry::Size;
use peek_document::{CanvasDocument, NodeId, NodeType};
use peek_ui::WorkspaceView;

/// One text node at world (100, 100), 300x200, with an identity viewport so world and pane
/// coordinates differ only by the pane origin.
///
/// The text is deliberately short: a text node sizes its font from its own height (200 units
/// here, so 124), and anything longer would trip the auto-grow and move the edges the resize
/// and drag tests aim at.
const ONE_NODE: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "text_aaaaaaaa",
        "type": "text",
        "position": { "x": 100, "y": 100 },
        "width": 300,
        "height": 200,
        "data": { "text": "hi" }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const NODE: &str = "text_aaaaaaaa";

fn open(cx: &mut TestAppContext, json: &str) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    let json = json.to_string();
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let document = CanvasDocument::from_json(&json).unwrap();
        let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, workspace.unwrap())
}

fn node_bounds(cx: &mut TestAppContext, handle: WindowHandle<Root>) -> Bounds<Pixels> {
    cx.update_window(handle.into(), |_, window, _| window.find(NODE).bounds())
        .unwrap()
}

fn inside(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    point(bounds.origin.x + px(x), bounds.origin.y + px(y))
}

/// A left drag with the secondary modifier held for every event of the gesture. The kit's
/// `Window::drag` sends unmodified events, and cmd is read from the press.
fn cmd_drag(visual: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    let modifiers = Modifiers::secondary_key();
    visual.simulate_event(MouseMoveEvent {
        position: from,
        pressed_button: None,
        modifiers,
    });
    visual.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: from,
        modifiers,
        click_count: 1,
        first_mouse: false,
    });
    visual.simulate_event(MouseMoveEvent {
        position: to,
        pressed_button: Some(MouseButton::Left),
        modifiers,
    });
    visual.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: to,
        modifiers,
        click_count: 1,
    });
}

fn world_position(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> (f64, f64) {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let node = document.node(&NodeId::from(NODE)).expect("node exists");
        (node.position.x, node.position.y)
    })
}

fn world_size(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Size {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .node(&NodeId::from(NODE))
            .expect("node exists")
            .size()
    })
}

fn selection(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<String> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .selected()
            .iter()
            .map(std::string::ToString::to_string)
            .collect()
    })
}

#[gpui_kit::test]
fn clicking_a_node_header_selects_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    assert!(selection(cx, &workspace).is_empty());

    let bounds = node_bounds(cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        let at = inside(bounds, 150.0, 20.0);
        window.drag(at, at, cx);
    })
    .unwrap();

    assert_eq!(selection(cx, &workspace), vec![NODE.to_string()]);
}

#[gpui_kit::test]
fn dragging_a_node_header_moves_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    let bounds = node_bounds(cx, handle);

    cx.update_window(handle.into(), |_, window, cx| {
        window.drag(inside(bounds, 150.0, 20.0), inside(bounds, 230.0, 60.0), cx);
    })
    .unwrap();

    // Zoom is 1, so screen pixels are world units.
    assert_eq!(world_position(cx, &workspace), (180.0, 140.0));
    assert_eq!(selection(cx, &workspace), vec![NODE.to_string()]);
}

#[gpui_kit::test]
fn dragging_a_node_body_selects_but_does_not_move_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    let bounds = node_bounds(cx, handle);
    let before = world_position(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.drag(
            inside(bounds, 150.0, 120.0),
            inside(bounds, 250.0, 160.0),
            cx,
        );
    })
    .unwrap();

    assert_eq!(
        world_position(cx, &workspace),
        before,
        "the body owns the pointer; only the header drags"
    );
}

/// Cmd turns the whole card into a drag handle, and the cursor says so before the press.
#[gpui_kit::test]
fn cmd_dragging_a_node_body_moves_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    let bounds = node_bounds(cx, handle);

    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    cmd_drag(
        &mut visual,
        inside(bounds, 150.0, 120.0),
        inside(bounds, 250.0, 160.0),
    );

    assert_eq!(world_position(cx, &workspace), (200.0, 140.0));
    assert_eq!(selection(cx, &workspace), vec![NODE.to_string()]);
}

#[gpui_kit::test]
fn holding_cmd_over_a_body_shows_the_grab_cursor(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    let bounds = node_bounds(cx, handle);

    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_event(MouseMoveEvent {
        position: inside(bounds, 150.0, 120.0),
        pressed_button: None,
        modifiers: Modifiers::secondary_key(),
    });

    assert_eq!(
        cx.update(|cx| workspace.read(cx).cursor(cx)),
        CursorStyle::OpenHand
    );
}

#[gpui_kit::test]
fn dragging_a_corner_resizes_and_clamps_to_the_minimum(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    let bounds = node_bounds(cx, handle);

    cx.update_window(handle.into(), |_, window, cx| {
        window.drag(
            inside(bounds, 298.0, 198.0),
            inside(bounds, 398.0, 298.0),
            cx,
        );
    })
    .unwrap();
    assert_eq!(world_size(cx, &workspace), Size::new(400.0, 300.0));

    // Now drag the same corner far past the minimum.
    let bounds = node_bounds(cx, handle);
    cx.update_window(handle.into(), |_, window, cx| {
        window.drag(inside(bounds, 398.0, 298.0), inside(bounds, 2.0, 2.0), cx);
    })
    .unwrap();
    assert_eq!(world_size(cx, &workspace), NodeType::Text.min_size());
}

/// The pointer's affordance is decided before the press: grab on the header, the matching
/// diagonal on a resize corner, and nothing of its own over the body.
#[gpui_kit::test]
fn hovering_a_node_shows_the_affordance_under_the_pointer(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    let bounds = node_bounds(cx, handle);

    let cursor = |cx: &mut TestAppContext, at: Point<Pixels>| {
        cx.update_window(handle.into(), |_, window, cx| {
            window.simulate_mouse_move(at, cx);
        })
        .unwrap();
        cx.update(|cx| workspace.read(cx).cursor(cx))
    };

    assert_eq!(
        cursor(cx, inside(bounds, 150.0, 20.0)),
        CursorStyle::OpenHand
    );
    assert_eq!(
        cursor(cx, inside(bounds, 150.0, 6.0)),
        CursorStyle::ResizeUpDown,
        "the header's own top edge still resizes"
    );
    assert_eq!(
        cursor(cx, inside(bounds, 298.0, 198.0)),
        CursorStyle::ResizeUpLeftDownRight
    );
    assert_eq!(
        cursor(cx, inside(bounds, 2.0, 100.0)),
        CursorStyle::ResizeLeftRight
    );
    assert_eq!(cursor(cx, inside(bounds, 150.0, 120.0)), CursorStyle::Arrow);
    assert_eq!(
        cursor(cx, point(px(4.0), px(4.0))),
        CursorStyle::Arrow,
        "off the node"
    );
}

#[gpui_kit::test]
fn wheel_over_a_node_still_pans_the_canvas(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    let before = cx.update(|cx| workspace.read(cx).camera(cx));

    cx.update_window(handle.into(), |_, window, cx| {
        window.scroll(NODE, ScrollDelta::Pixels(point(px(30.0), px(50.0))), cx);
    })
    .unwrap();

    let after = cx.update(|cx| workspace.read(cx).camera(cx));
    assert!(
        (after.pan.x - (before.pan.x + 30.0)).abs() < 0.01,
        "wheel over a node pans: {after:?}"
    );
}

#[gpui_kit::test]
fn backspace_deletes_the_selection_and_undo_restores_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    let bounds = node_bounds(cx, handle);

    cx.update_window(handle.into(), |_, window, cx| {
        let at = inside(bounds, 150.0, 12.0);
        window.drag(at, at, cx);
        window.press("backspace", cx);
    })
    .unwrap();
    assert_eq!(node_count(cx, &workspace), 0);

    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-z", cx))
        .unwrap();
    assert_eq!(node_count(cx, &workspace), 1);
}

#[gpui_kit::test]
fn pressing_t_then_clicking_places_a_text_node_at_the_pointer(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);
    assert_eq!(node_count(cx, &workspace), 1);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("t", cx);
        let at = point(px(700.0), px(600.0));
        window.drag(at, at, cx);
    })
    .unwrap();

    assert_eq!(node_count(cx, &workspace), 2);
    let placed = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.selected().iter().next().cloned()
    });
    assert!(
        placed.is_some_and(|id| id.as_str().starts_with("text_")),
        "the new node is selected"
    );
}

fn node_count(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> usize {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        document.read(cx).nodes().len()
    })
}

/// One query-error node at world (100, 100), the reference's 400x300 default, carrying a
/// multi-line Postgres error. Its id is `<query id>-error`, as `NodeId::error_of` mints it.
const QUERY_ERROR_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "query_bbbbbbbb-error",
        "type": "query-error",
        "position": { "x": 100, "y": 100 },
        "width": 400,
        "height": 300,
        "data": {
          "queryNodeId": "query_bbbbbbbb",
          "query": "select * from usrs",
          "message": "ERROR: relation \"usrs\" does not exist\nLINE 1: select * from usrs\n                      ^"
        }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const QUERY_ERROR: &str = "query_bbbbbbbb-error";

/// The same words the fixture hands the node, unescaped.
const DATABASE_MESSAGE: &str =
    "ERROR: relation \"usrs\" does not exist\nLINE 1: select * from usrs\n                      ^";

#[gpui_kit::test]
fn a_query_error_node_renders_the_database_message(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, QUERY_ERROR_DOCUMENT);

    let (label, message, shell) = cx
        .update_window(handle.into(), |_, window, _| {
            let message = window.find(SharedString::from(format!("{QUERY_ERROR}-message")));
            (
                message.label().map(str::to_string),
                message.bounds(),
                window.find(QUERY_ERROR).bounds(),
            )
        })
        .unwrap();

    assert_eq!(
        label.as_deref(),
        Some(DATABASE_MESSAGE),
        "the body carries the database's own words, newlines and all"
    );
    assert!(
        message.size.height > px(0.0) && shell.contains(&message.origin),
        "the message fills the shell below the header: {message:?} inside {shell:?}"
    );
}

/// One table-definition node with a column per type category, including spellings that fall
/// through to `other`.
const TABLE_DEFINITION_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "query_cccccccc",
        "type": "table-definition",
        "position": { "x": 100, "y": 100 },
        "width": 450,
        "height": 280,
        "data": {
          "table": "users",
          "columns": [
            ["id", "int4"],
            ["email", "character varying(255)"],
            ["created_at", "timestamp with time zone"],
            ["prefs", "jsonb"],
            ["external_id", "uuid"],
            ["is_active", "bool"]
          ]
        }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const TABLE_DEFINITION: &str = "query_cccccccc";

const COLUMNS: [(&str, &str); 6] = [
    ("id", "int4"),
    ("email", "character varying(255)"),
    ("created_at", "timestamp with time zone"),
    ("prefs", "jsonb"),
    ("external_id", "uuid"),
    ("is_active", "bool"),
];

#[gpui_kit::test]
fn a_table_definition_node_renders_a_row_per_column(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, TABLE_DEFINITION_DOCUMENT);

    let (shell, rows) = cx
        .update_window(handle.into(), |_, window, _| {
            let rows: Vec<(Option<String>, Bounds<Pixels>)> = COLUMNS
                .iter()
                .map(|(name, _)| {
                    let row = window.find(*name);
                    (row.label().map(str::to_string), row.bounds())
                })
                .collect();
            (window.find(TABLE_DEFINITION).bounds(), rows)
        })
        .unwrap();

    let mut previous_bottom = shell.origin.y;
    for ((name, column_type), (label, bounds)) in COLUMNS.iter().zip(rows) {
        assert_eq!(
            label.as_deref(),
            Some(format!("{name} {column_type}").as_str()),
            "the row carries both cells"
        );
        assert!(
            bounds.size.height > px(0.0) && shell.contains(&bounds.origin),
            "{name} is laid out inside the shell: {bounds:?} inside {shell:?}"
        );
        assert!(
            bounds.origin.y >= previous_bottom,
            "{name} follows the row above it, in document order"
        );
        previous_bottom = bounds.bottom();
    }
}

/// One draw node holding a recorded stroke, laid out the way `useDrawTool.ts` commits one:
/// points relative to the origin, inset by `strokeWidth * 2` on every side.
const DRAW_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "draw_dddddddd",
        "type": "draw",
        "position": { "x": 100, "y": 100 },
        "width": 116,
        "height": 66,
        "data": {
          "strokeWidth": 4,
          "color": "var(--pk-fg)",
          "points": [
            [8, 50, 0.5], [18, 44, 0.5], [29, 37, 0.5], [41, 31, 0.52],
            [54, 26, 0.55], [66, 24, 0.6], [78, 26, 0.58], [88, 32, 0.5],
            [96, 41, 0.42], [102, 50, 0.35], [106, 57, 0.3], [108, 58, 0.2]
          ]
        }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const DRAW: &str = "draw_dddddddd";

#[gpui_kit::test]
fn a_draw_node_renders_its_stroke_and_nothing_else(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, DRAW_DOCUMENT);

    // Reaching this at all means the stroke tessellated and painted: the path is built during
    // the frame `open` draws, and a failure there would take the window down with it.
    let (bounds, label) = cx
        .update_window(handle.into(), |_, window, _| {
            let node = window.find(DRAW);
            (node.bounds(), node.label().map(str::to_string))
        })
        .unwrap();

    // Zoom is 1 and the viewport is at the origin, so the node covers its own world rect.
    assert_eq!(bounds.size, size(px(116.0), px(66.0)));
    assert_eq!(
        label, None,
        "a draw node is its stroke alone: no header, no kind label, no title"
    );
}

/// One empty text node, which `TextNode.tsx` opens straight into edit mode.
const EMPTY_TEXT_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "text_aaaaaaaa",
        "type": "text",
        "position": { "x": 100, "y": 100 },
        "width": 300,
        "height": 60,
        "data": { "text": "" }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

fn node_text(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> String {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        match &document
            .node(&NodeId::from(NODE))
            .expect("node exists")
            .kind
        {
            peek_document::NodeKind::Text(data) => data.text.clone(),
            kind => panic!("not a text node: {kind:?}"),
        }
    })
}

/// The card a text node draws instead of the shared shell.
fn text_card(cx: &mut TestAppContext, handle: WindowHandle<Root>) -> Option<String> {
    cx.update_window(handle.into(), |_, window, _| {
        window
            .find(SharedString::from(format!("{NODE}-text")))
            .label()
            .map(str::to_string)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_text_node_shows_its_text_and_a_hint_when_empty(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, ONE_NODE);
    assert_eq!(text_card(cx, handle).as_deref(), Some("hi"));
}

#[gpui_kit::test]
fn double_clicking_a_text_node_enters_edit_mode(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);

    // Nothing in the node has focus yet, so keystrokes are the canvas'.
    cx.update_window(handle.into(), |_, window, cx| window.input("x", cx))
        .unwrap();
    assert_eq!(node_text(cx, &workspace), "hi");

    cx.update_window(handle.into(), |_, window, cx| {
        window.double_click(NODE, cx);
        window.input("x", cx);
    })
    .unwrap();
    assert_eq!(
        node_text(cx, &workspace),
        "hix",
        "the second click opens the editor with the caret at the end"
    );
}

#[gpui_kit::test]
fn typing_in_a_text_node_writes_through_to_the_document(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);

    cx.update_window(handle.into(), |_, window, cx| {
        window.double_click(NODE, cx);
        window.input(" there", cx);
    })
    .unwrap();

    assert_eq!(node_text(cx, &workspace), "hi there");
    assert_eq!(
        text_card(cx, handle).as_deref(),
        Some("hi there"),
        "and the card renders what the document now holds"
    );
}

#[gpui_kit::test]
fn enter_leaves_edit_mode_and_deselects(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);

    cx.update_window(handle.into(), |_, window, cx| {
        window.double_click(NODE, cx);
        window.input("!", cx);
        window.press("enter", cx);
    })
    .unwrap();
    assert_eq!(node_text(cx, &workspace), "hi!");
    assert!(
        selection(cx, &workspace).is_empty(),
        "`Enter` blurs and deselects, as `TextNode.tsx` does"
    );

    cx.update_window(handle.into(), |_, window, cx| window.input("?", cx))
        .unwrap();
    assert_eq!(
        node_text(cx, &workspace),
        "hi!",
        "the editor is closed, so the keystroke is the canvas'"
    );
}

#[gpui_kit::test]
fn an_empty_text_node_starts_editable(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_TEXT_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| window.input("note", cx))
        .unwrap();

    assert_eq!(
        node_text(cx, &workspace),
        "note",
        "a node placed empty is typable without a double-click"
    );
}

#[gpui_kit::test]
fn a_text_node_grows_to_fit_its_widest_line(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_TEXT_DOCUMENT);
    let before = world_size(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.input("a line far wider than three hundred units", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let after = world_size(cx, &workspace);
    assert!(
        after.width > before.width,
        "the node widens to fit: {before:?} -> {after:?}"
    );
    assert!(
        (after.height - before.height).abs() < f64::EPSILON,
        "only the width grows"
    );
}

/// One barchart node holding two numeric series over a string category, the shape the result
/// node's `useChartSync` pushes in, plus one empty chart for the "No results" state.
const BARCHART_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [
        {
          "id": "query_eeeeeeee-result-0-chart",
          "type": "barchart",
          "position": { "x": 100, "y": 100 },
          "width": 460,
          "height": 290,
          "data": {
            "chartType": "bar",
            "data": [
              { "customer_name": "Vicosight", "total_quotes": 142, "signed_quotes": 12 },
              { "customer_name": "Learnster", "total_quotes": 18, "signed_quotes": 4 },
              { "customer_name": "Plock", "total_quotes": 61, "signed_quotes": 33 }
            ]
          }
        },
        {
          "id": "query_ffffffff-result-0-chart",
          "type": "barchart",
          "position": { "x": 700, "y": 100 },
          "width": 460,
          "height": 290,
          "data": { "data": [] }
        }
      ],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const BARCHART: &str = "query_eeeeeeee-result-0-chart";
const EMPTY_BARCHART: &str = "query_ffffffff-result-0-chart";

fn chart_type(
    cx: &mut TestAppContext,
    workspace: &Entity<WorkspaceView>,
) -> Option<peek_document::ChartType> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let node = document.node(&NodeId::from(BARCHART)).expect("node exists");
        <peek_document::BarChartData as peek_document::NodeData>::get(&node.kind)
            .expect("barchart data")
            .chart_type
    })
}

#[gpui_kit::test]
fn the_chart_type_toggle_writes_chart_type_to_the_document(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, BARCHART_DOCUMENT);
    assert_eq!(
        chart_type(cx, &workspace),
        Some(peek_document::ChartType::Bar)
    );

    // Each switch is followed by a frame, so the line and area painters run for real.
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(SharedString::from(format!("{BARCHART}-Line")), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(
        chart_type(cx, &workspace),
        Some(peek_document::ChartType::Line)
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(SharedString::from(format!("{BARCHART}-Area")), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(
        chart_type(cx, &workspace),
        Some(peek_document::ChartType::Area)
    );

    // The spelling the TypeScript app reads back.
    let json = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.inner().to_json()
    });
    assert!(
        json.contains(r#""chartType":"area""#),
        "the on-disk spelling stays kebab-free lowercase: {json}"
    );
}

#[gpui_kit::test]
fn each_chart_type_switch_is_its_own_undo_step(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, BARCHART_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(SharedString::from(format!("{BARCHART}-Line")), cx);
        window.press("cmd-z", cx);
    })
    .unwrap();

    assert_eq!(
        chart_type(cx, &workspace),
        Some(peek_document::ChartType::Bar)
    );
}

#[gpui_kit::test]
fn an_empty_barchart_says_so_instead_of_drawing_a_chart(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, BARCHART_DOCUMENT);

    let (empty, populated) = cx
        .update_window(handle.into(), |_, window, _| {
            (
                window
                    .find(SharedString::from(format!("{EMPTY_BARCHART}-empty")))
                    .label()
                    .map(str::to_string),
                window.try_find(SharedString::from(format!("{BARCHART}-empty"))),
            )
        })
        .unwrap();

    assert_eq!(empty.as_deref(), Some("No results"));
    assert!(
        populated.is_none(),
        "a chart with rows draws the chart, not the empty message"
    );
}

/// One variable node holding every state the table can be in — a good row, a row that repeats
/// its name, a name outside `VARIABLE_NAME_RE`, and a list — plus a query node for the global
/// toggle to connect itself to.
const VARIABLE_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [
        {
          "id": "variable_bbbbbbbb",
          "type": "variable",
          "position": { "x": 100, "y": 100 },
          "width": 360,
          "height": 280,
          "data": {
            "rows": [
              { "name": "customer", "value": "Plock" },
              { "name": "customer", "value": "Learnster" },
              { "name": "2nd", "value": "x" },
              { "name": "ids", "value": ["1", "2"] }
            ]
          }
        },
        {
          "id": "query_cccccccc",
          "type": "query",
          "position": { "x": 600, "y": 100 },
          "width": 400,
          "height": 200,
          "data": { "query": "select 1" }
        }
      ],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

/// One variable node with the single blank row `makeNode` gives a fresh one.
const EMPTY_VARIABLE_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "variable_bbbbbbbb",
        "type": "variable",
        "position": { "x": 100, "y": 100 },
        "width": 360,
        "height": 200,
        "data": { "rows": [{ "name": "", "value": "" }] }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const VARIABLE: &str = "variable_bbbbbbbb";
const QUERY: &str = "query_cccccccc";

fn variable_rows(
    cx: &mut TestAppContext,
    workspace: &Entity<WorkspaceView>,
) -> Vec<peek_document::VariableRow> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let node = document.node(&NodeId::from(VARIABLE)).expect("node exists");
        <peek_document::VariableData as peek_document::NodeData>::get(&node.kind)
            .expect("variable data")
            .rows
            .clone()
    })
}

fn name_label(cx: &mut TestAppContext, handle: WindowHandle<Root>, key: usize) -> Option<String> {
    cx.update_window(handle.into(), |_, window, _| {
        window
            .find(("variable-name", key))
            .label()
            .map(str::to_string)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_malformed_or_duplicated_variable_name_is_flagged(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, VARIABLE_DOCUMENT);

    assert_eq!(
        name_label(cx, handle, 0).as_deref(),
        Some("Another row already uses this name")
    );
    assert_eq!(
        name_label(cx, handle, 1).as_deref(),
        Some("Another row already uses this name"),
        "both halves of the collision are flagged, as `nameCounts[name] > 1` does"
    );
    assert_eq!(
        name_label(cx, handle, 2).as_deref(),
        Some("Starts with a letter or underscore, then letters, digits or underscores")
    );
    assert_eq!(name_label(cx, handle, 3).as_deref(), Some("Variable name"));
}

#[gpui_kit::test]
fn add_variable_appends_a_blank_row_and_the_trash_removes_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, VARIABLE_DOCUMENT);
    assert_eq!(variable_rows(cx, &workspace).len(), 4);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("variable-add-row", cx);
    })
    .unwrap();
    let rows = variable_rows(cx, &workspace);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[4].name, "");

    // The row added last is keyed 4: keys are handed out in order and outlive their neighbours.
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-remove-row", 4usize), cx);
    })
    .unwrap();
    assert_eq!(variable_rows(cx, &workspace).len(), 4);

    // Removing from the middle takes that row, not the one that slid into its place.
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-remove-row", 1usize), cx);
    })
    .unwrap();
    let names: Vec<String> = variable_rows(cx, &workspace)
        .into_iter()
        .map(|row| row.name)
        .collect();
    assert_eq!(names, vec!["customer", "2nd", "ids"]);
}

#[gpui_kit::test]
fn the_last_row_cannot_be_removed(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_VARIABLE_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-remove-row", 0usize), cx);
    })
    .unwrap();

    let rows = variable_rows(cx, &workspace);
    assert_eq!(rows.len(), 1, "`removeRow` keeps a row to type into");
    assert_eq!(rows[0].name, "");
    assert_eq!(
        rows[0].value,
        peek_document::VariableValue::One(String::new())
    );
}

#[gpui_kit::test]
fn the_list_toggle_round_trips_through_variable_value(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, VARIABLE_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-list-toggle", 0usize), cx);
    })
    .unwrap();
    assert_eq!(
        variable_rows(cx, &workspace)[0].value,
        peek_document::VariableValue::Many(vec!["Plock".to_string()]),
        "a one-line value splits into a list of one, which is a different document"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-list-toggle", 0usize), cx);
    })
    .unwrap();
    assert_eq!(
        variable_rows(cx, &workspace)[0].value,
        peek_document::VariableValue::One("Plock".to_string())
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-list-toggle", 3usize), cx);
    })
    .unwrap();
    assert_eq!(
        variable_rows(cx, &workspace)[3].value,
        peek_document::VariableValue::One("1\n2".to_string()),
        "`toggleArrayMode` joins a list on newlines"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-list-toggle", 3usize), cx);
    })
    .unwrap();
    assert_eq!(
        variable_rows(cx, &workspace)[3].value,
        peek_document::VariableValue::Many(vec!["1".to_string(), "2".to_string()])
    );

    // `VariableValue` is untagged, so the shape has to survive the round trip on disk too.
    let json = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document.inner().to_json()
    });
    assert!(
        json.contains(r#""value":["1","2"]"#),
        "a list stays a JSON array: {json}"
    );
}

#[gpui_kit::test]
fn an_empty_value_becomes_an_empty_list(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_VARIABLE_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-list-toggle", 0usize), cx);
    })
    .unwrap();

    assert_eq!(
        variable_rows(cx, &workspace)[0].value,
        peek_document::VariableValue::Many(Vec::new()),
        "not a list holding one empty string"
    );
}

#[gpui_kit::test]
fn the_globe_connects_the_node_to_every_query_on_the_page(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, VARIABLE_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click("variable-global", cx);
    })
    .unwrap();

    let (global, edges) = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let node = document.node(&NodeId::from(VARIABLE)).expect("node exists");
        let global = <peek_document::VariableData as peek_document::NodeData>::get(&node.kind)
            .expect("variable data")
            .is_global;
        let edges: Vec<(String, String)> = document
            .edges()
            .iter()
            .map(|edge| (edge.source.to_string(), edge.target.to_string()))
            .collect();
        (global, edges)
    });

    assert_eq!(global, Some(true));
    assert_eq!(edges, vec![(VARIABLE.to_string(), QUERY.to_string())]);

    // Turning it off leaves the edges alone, as the reference does.
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("variable-global", cx);
    })
    .unwrap();
    let global = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let node = document.node(&NodeId::from(VARIABLE)).expect("node exists");
        <peek_document::VariableData as peek_document::NodeData>::get(&node.kind)
            .expect("variable data")
            .is_global
    });
    assert_eq!(global, Some(false));
}

#[gpui_kit::test]
fn typing_a_name_writes_through_to_the_document(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_VARIABLE_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-name", 0usize), cx);
        window.input("limit", cx);
    })
    .unwrap();

    assert_eq!(variable_rows(cx, &workspace)[0].name, "limit");
}

#[gpui_kit::test]
fn pasting_several_lines_turns_the_row_into_a_list(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_VARIABLE_DOCUMENT);
    cx.update(|cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
            "alice\n\nbob\n".to_string(),
        ));
    });

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-value-input", 0usize), cx);
        window.press("cmd-v", cx);
    })
    .unwrap();

    assert_eq!(
        variable_rows(cx, &workspace)[0].value,
        peek_document::VariableValue::Many(vec!["alice".to_string(), "bob".to_string()]),
        "a pasted column is a list, blank lines dropped, and the input never flattens it"
    );
}

#[gpui_kit::test]
fn pasting_a_single_line_stays_a_single_value(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_VARIABLE_DOCUMENT);
    cx.update(|cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("alice".to_string()));
    });

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-value-input", 0usize), cx);
        window.press("cmd-v", cx);
    })
    .unwrap();

    assert_eq!(
        variable_rows(cx, &workspace)[0].value,
        peek_document::VariableValue::One("alice".to_string()),
        "one line pastes into the input as it always did"
    );
}

#[gpui_kit::test]
fn the_chip_opens_and_closes_the_list_editor(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, VARIABLE_DOCUMENT);

    let closed = cx
        .update_window(handle.into(), |_, window, _| {
            window.try_find(("variable-list-editor", 3usize)).is_some()
        })
        .unwrap();
    assert!(!closed, "a list starts collapsed behind its chip");

    let label = cx
        .update_window(handle.into(), |_, window, cx| {
            window.click(("variable-list-chip", 3usize), cx);
            window
                .find(("variable-list-editor", 3usize))
                .label()
                .map(str::to_string)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("@ids"));

    let reclosed = cx
        .update_window(handle.into(), |_, window, cx| {
            window.click(("variable-list-chip", 3usize), cx);
            window.try_find(("variable-list-editor", 3usize)).is_some()
        })
        .unwrap();
    assert!(!reclosed, "the chip toggles");
}

#[gpui_kit::test]
fn undo_puts_the_typed_name_back_in_the_field(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_VARIABLE_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(("variable-name-input", 0usize), cx);
        window.input("limit", cx);
    })
    .unwrap();
    // The change reaches the document through a subscription, which runs once the window
    // update that raised it has finished.
    cx.run_until_parked();
    assert_eq!(variable_rows(cx, &workspace)[0].name, "limit");

    cx.update_window(handle.into(), |_, window, cx| {
        // Escape leaves the `Input` context, which is what puts the canvas' own bindings —
        // `cmd-z` among them — back in scope without leaving the node.
        window.press("escape", cx);
        window.press("cmd-z", cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(variable_rows(cx, &workspace)[0].name, "");
    let value = cx
        .update_window(handle.into(), |_, window, _| {
            window
                .find(("variable-name-input", 0usize))
                .value()
                .map(str::to_string)
        })
        .unwrap();
    assert_eq!(
        value.as_deref(),
        Some(""),
        "the field follows the document back, as `useSyncedFieldValue` does"
    );
}

#[gpui_kit::test]
fn undoing_the_typing_takes_the_width_it_caused_with_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, EMPTY_TEXT_DOCUMENT);
    let before = world_size(cx, &workspace);

    cx.update_window(handle.into(), |_, window, cx| {
        window.input("a line far wider than three hundred units", cx);
        window.render_frame(cx);
        // The editor owns `cmd-z` while it holds focus; hand the canvas its keys back.
        window.press("escape", cx);
    })
    .unwrap();
    let grown = world_size(cx, &workspace);
    assert!(grown.width > before.width, "the node widened while typing");

    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-z", cx))
        .unwrap();

    assert_eq!(node_text(cx, &workspace), "");
    assert_eq!(
        world_size(cx, &workspace),
        before,
        "the grow was intrinsic, so it folded into the typing instead of being its own step"
    );
}

// ---- Query ------------------------------------------------------------------------------

/// One query node, wide enough that the footer clears the bottom resize band.
const SQL_DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "query_dddddddd",
        "type": "query",
        "position": { "x": 100, "y": 100 },
        "width": 360,
        "height": 240,
        "data": { "query": "select 1" }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

const SQL_NODE: &str = "query_dddddddd";

/// The footer's Run button. `gpui_component::Button` does not report `disabled` through
/// accessibility, so its label is the observable: it reads "Run", "Running…" or "Run unbounded"
/// depending on the state the node is in.
#[gpui_kit::test]
fn the_run_button_offers_itself_even_without_a_connection(cx: &mut TestAppContext) {
    let (handle, _) = open(cx, SQL_DOCUMENT);
    let run = SharedString::from(format!("{SQL_NODE}-run"));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(run).label(),
            Some("Run"),
            "the node still explains what it would do; the tooltip says why it cannot"
        );
    })
    .unwrap();
}

/// Pressing Run with no connection must not mark the node as running: the flag is persisted,
/// so a node stuck at `isRunning: true` would be written to disk that way.
#[gpui_kit::test]
fn a_refused_run_does_not_leave_the_node_running(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, SQL_DOCUMENT);
    let run = SharedString::from(format!("{SQL_NODE}-run"));
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(run, cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(sql_data(cx, &workspace).is_running, None);
}

fn sql_body() -> SharedString {
    SharedString::from(format!("{SQL_NODE}-editor"))
}

fn sql_data(
    cx: &mut TestAppContext,
    workspace: &Entity<WorkspaceView>,
) -> peek_document::QueryData {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let node = document
            .node(&NodeId::from(SQL_NODE))
            .expect("query exists");
        match &node.kind {
            peek_document::NodeKind::Query(data) => data.clone(),
            other => panic!("expected a query node, got {other:?}"),
        }
    })
}

#[gpui_kit::test]
fn typing_in_a_query_editor_writes_through_to_the_document(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, SQL_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(sql_body(), cx);
        window.input(" + 1", cx);
    })
    .unwrap();

    assert_eq!(sql_data(cx, &workspace).query, "select 1 + 1");
}

#[gpui_kit::test]
fn formatting_a_query_rewrites_it_in_the_document(cx: &mut TestAppContext) {
    let json = SQL_DOCUMENT.replace(
        r#""query": "select 1""#,
        r#""query": "select a,b from users where id = 1""#,
    );
    let (handle, workspace) = open(cx, &json);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(sql_body(), cx);
        window.press("cmd-s", cx);
    })
    .unwrap();

    let query = sql_data(cx, &workspace).query;
    assert!(
        query.contains("SELECT"),
        "Query::Format uppercases keywords, got {query:?}"
    );
    assert!(
        query.contains('\n'),
        "and breaks the statement across lines, got {query:?}"
    );
}

#[gpui_kit::test]
fn formatting_keeps_peek_variables_intact(cx: &mut TestAppContext) {
    let json = SQL_DOCUMENT.replace(
        r#""query": "select 1""#,
        r#""query": "select * from users where id = @user_id""#,
    );
    let (handle, workspace) = open(cx, &json);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(sql_body(), cx);
        window.press("cmd-s", cx);
    })
    .unwrap();

    let query = sql_data(cx, &workspace).query;
    assert!(
        query.contains("@user_id"),
        "the variable must survive formatting, got {query:?}"
    );
    assert!(
        !query.contains("__pkvar"),
        "and its placeholder must not leak"
    );
}

#[gpui_kit::test]
fn the_live_toggle_round_trips_live_interval_ms(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, SQL_DOCUMENT);
    assert_eq!(sql_data(cx, &workspace).live_interval_ms, None);

    let toggle = SharedString::from(format!("{SQL_NODE}-live"));
    cx.update_window(handle.into(), |_, window, cx| {
        window.click(toggle.clone(), cx);
    })
    .unwrap();
    assert_eq!(
        sql_data(cx, &workspace).live_interval_ms,
        Some(peek_document::LiveInterval::EveryMs(10_000)),
        "the toggle writes the interval the reference persists"
    );

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(toggle.clone(), cx);
    })
    .unwrap();
    assert_eq!(sql_data(cx, &workspace).live_interval_ms, None);
}

#[gpui_kit::test]
fn pressing_q_then_clicking_places_a_query_node(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, SQL_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("q", cx);
        let at = point(px(800.0), px(600.0));
        window.drag(at, at, cx);
    })
    .unwrap();

    let queries = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        document
            .nodes()
            .iter()
            .filter(|node| node.node_type() == Some(NodeType::Query))
            .count()
    });
    assert_eq!(queries, 2, "the tool places a second query node");
}

/// The drag sizes the node itself rather than a preview rectangle, and the release hands a
/// query straight to its editor, so typing lands in the SQL without a further click.
#[gpui_kit::test]
fn dragging_the_query_tool_sizes_the_node_and_focuses_its_editor(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, SQL_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("q", cx);
        window.drag(point(px(300.0), px(200.0)), point(px(800.0), px(600.0)), cx);
        window.input("select 1", cx);
    })
    .unwrap();

    let placed = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let id = document
            .selected()
            .iter()
            .next()
            .cloned()
            .expect("the placed node is selected");
        document.node(&id).cloned().expect("the placed node exists")
    });
    assert_eq!(
        placed.size(),
        Size::new(500.0, 400.0),
        "the viewport is identity, so the dragged pixels are the node's world size"
    );
    let peek_document::NodeKind::Query(data) = &placed.kind else {
        panic!("expected a query node, got {:?}", placed.kind);
    };
    assert_eq!(
        data.query, "select 1",
        "the editor took focus when the drag ended"
    );
}

/// The whole drag is one undo step: the creation and every size it passed through.
#[gpui_kit::test]
fn undoing_a_placement_drag_removes_the_node_it_created(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);

    cx.update_window(handle.into(), |_, window, cx| {
        window.press("t", cx);
        window.drag(point(px(300.0), px(200.0)), point(px(800.0), px(600.0)), cx);
        // A node placed empty opens its editor, which owns `cmd-z` while it holds focus.
        window.press("escape", cx);
    })
    .unwrap();
    assert_eq!(node_count(cx, &workspace), 2);

    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-z", cx))
        .unwrap();
    assert_eq!(
        node_count(cx, &workspace),
        1,
        "one undo takes the drag back, not one size at a time"
    );
}

/// Escape mid-drag drops the node the drag had already created, leaving nothing to undo.
#[gpui_kit::test]
fn escaping_a_placement_drag_drops_the_node_and_leaves_no_undo_step(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, ONE_NODE);

    cx.update_window(handle.into(), |_, window, cx| window.press("q", cx))
        .unwrap();

    // Clear of the document's one node, so the press reaches the canvas rather than an editor.
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(700.0), px(450.0)),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    visual.simulate_event(MouseMoveEvent {
        position: point(px(1000.0), px(700.0)),
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::default(),
    });
    assert_eq!(
        node_count(cx, &workspace),
        2,
        "the drag placed a node while the pointer was still down"
    );

    cx.update_window(handle.into(), |_, window, cx| window.press("escape", cx))
        .unwrap();
    assert_eq!(node_count(cx, &workspace), 1);

    cx.update_window(handle.into(), |_, window, cx| window.press("cmd-z", cx))
        .unwrap();
    assert_eq!(
        node_count(cx, &workspace),
        1,
        "the cancelled placement left no undo step to bring it back"
    );
}

#[gpui_kit::test]
fn escape_hands_focus_back_so_undo_reaches_the_canvas(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, SQL_DOCUMENT);

    cx.update_window(handle.into(), |_, window, cx| {
        window.click(sql_body(), cx);
        window.input(" + 1", cx);
        window.press("escape", cx);
        window.press("cmd-z", cx);
    })
    .unwrap();

    assert_eq!(
        sql_data(cx, &workspace).query,
        "select 1",
        "undo must reach the canvas once the editor gives focus back"
    );
}
