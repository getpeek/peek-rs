//! The agent node, driven through real key and pointer dispatch.
//!
//! No test here spawns an agent: `mark_configured_for_test` declares a provider with no session
//! behind it, and the streaming paths are driven by feeding `AcpUpdate`s straight into the view.
//! That is deliberate — a suite that shelled out to `npx` would be neither fast nor honest.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_acp::AcpUpdate;
use peek_document::{AgentData, AgentMessage, AgentProvider, CanvasDocument, NodeData, NodeId};

use super::backend::Agents;
use super::view::AgentView;
use crate::WorkspaceView;

const DOCUMENT: &str = r#"{
  "version": 1,
  "activePageId": "page_test0001",
  "pageOrder": ["page_test0001"],
  "pages": {
    "page_test0001": {
      "id": "page_test0001",
      "name": "Page 1",
      "nodes": [{
        "id": "agent_aaaaaaaa",
        "type": "agent",
        "position": { "x": 100, "y": 100 },
        "width": 540,
        "height": 400,
        "data": { "query": "", "messages": [] }
      }],
      "edges": [],
      "viewport": { "x": 0, "y": 0, "zoom": 1 }
    }
  }
}"#;

fn agent_node() -> NodeId {
    NodeId::from("agent_aaaaaaaa")
}

fn open(cx: &mut TestAppContext, configured: bool) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        crate::init(&config, cx);
        if configured {
            Agents::mark_configured_for_test(AgentProvider::Acp, cx);
        }
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let document = CanvasDocument::from_json(DOCUMENT).unwrap();
        let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, workspace.unwrap())
}

/// Seeds the document's transcript before the node is first rendered.
fn with_messages(
    cx: &mut TestAppContext,
    messages: Vec<AgentMessage>,
) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    let (handle, workspace) = open(cx, true);
    cx.update(|cx| {
        workspace.read(cx).document(cx).update(cx, |document, _| {
            document.update_data::<AgentData>(&agent_node(), |data| data.messages = messages);
        });
    });
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, workspace)
}

fn view(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Entity<AgentView> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .agent_view(&agent_node(), cx)
            .expect("the agent node has retained state")
    })
}

fn committed(cx: &mut TestAppContext, workspace: &Entity<WorkspaceView>) -> Vec<AgentMessage> {
    cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        let node = document.node(&agent_node()).expect("the node is there");
        AgentData::get(&node.kind)
            .expect("an agent node")
            .messages
            .clone()
    })
}

fn message(kind: &str, text: &str) -> AgentMessage {
    AgentMessage::new(kind, text.to_string(), 1)
}

#[gpui_kit::test]
fn a_persisted_conversation_renders_its_blocks(cx: &mut TestAppContext) {
    let (_handle, workspace) = with_messages(
        cx,
        vec![
            message("system", "you are a helpful agent"),
            message("user", "how many users?"),
            message("assistant", "Let me check."),
        ],
    );

    let node = view(cx, &workspace);
    let rows = cx.update(|cx| node.read(cx).row_count());
    assert_eq!(rows, 2, "the seeded system prompt is not part of the chat");
}

/// The node has to render without a backend, and say what to do about it — an agent node in a
/// document opened on a machine with no `ai` block is the common case, not an error.
#[gpui_kit::test]
fn with_no_backend_the_node_explains_what_to_add(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, false);

    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("agent_aaaaaaaa-body").is_some());
        assert!(
            window.try_find("agent_aaaaaaaa-composer").is_none(),
            "there is nowhere to send a message, so there is no box to type one in"
        );
    })
    .unwrap();

    assert!(committed(cx, &workspace).is_empty());
}

#[gpui_kit::test]
fn a_configured_node_offers_a_composer(cx: &mut TestAppContext) {
    let (handle, _workspace) = open(cx, true);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("agent_aaaaaaaa-composer").is_some());
    })
    .unwrap();
}

/// Streaming text is a preview until the transcript reaches a boundary, so a chunk costs a
/// repaint and not a row — and half a sentence never lands in the document.
#[gpui_kit::test]
fn streamed_chunks_preview_and_only_become_a_row_at_a_boundary(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx, true);
    let view = view(cx, &workspace);

    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.pump(AcpUpdate::MessageChunk("Counting ".to_string()), cx);
            view.pump(AcpUpdate::MessageChunk("the users.".to_string()), cx);
        });
    });

    let (rows, preview) = cx.update(|cx| {
        let view = view.read(cx);
        (view.row_count(), view.preview().to_string())
    });
    assert_eq!(rows, 0, "nothing has been committed yet");
    assert_eq!(preview, "Counting the users.");

    // A tool call is a boundary: the answer so far flushes into a row of its own.
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.pump(
                AcpUpdate::ToolCall(peek_acp::ToolCallUpdate {
                    tool_call_id: "t1".to_string(),
                    title: Some("Read schema".to_string()),
                    ..peek_acp::ToolCallUpdate::default()
                }),
                cx,
            );
        });
    });

    let rows = cx.update(|cx| view.read(cx).row_count());
    assert_eq!(rows, 2, "the flushed answer, then the tool");
    assert!(
        committed(cx, &workspace).is_empty(),
        "still nothing in the document until the turn ends"
    );

    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// A turn is one action, so it commits once and undoes once. Anything finer would make undo
/// walk backwards through a conversation a token at a time.
#[gpui_kit::test]
fn finishing_a_turn_commits_it_in_one_undo_step(cx: &mut TestAppContext) {
    let (_handle, workspace) = open(cx, true);
    let view = view(cx, &workspace);

    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.stage(AgentMessage::new("user", "hi".to_string(), 1), cx);
            view.pump(AcpUpdate::MessageChunk("hello".to_string()), cx);
            view.finish_turn(cx);
        });
    });

    let messages = committed(cx, &workspace);
    assert_eq!(messages.len(), 2, "the question and the answer");
    assert_eq!(messages[0].kind, "user");
    assert_eq!(messages[1].kind, "assistant");

    cx.update(|cx| {
        workspace
            .read(cx)
            .document(cx)
            .update(cx, |document, _| assert!(document.undo()));
    });
    assert!(
        committed(cx, &workspace).is_empty(),
        "one press took the turn"
    );
}

/// A tool row is patched in place as its status changes, rather than appended again — otherwise
/// a long-running tool leaves a trail of duplicates.
#[gpui_kit::test]
fn a_tool_row_is_patched_in_place_as_it_progresses(cx: &mut TestAppContext) {
    let (_handle, workspace) = open(cx, true);
    let view = view(cx, &workspace);

    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.pump(
                AcpUpdate::ToolCall(peek_acp::ToolCallUpdate {
                    tool_call_id: "t1".to_string(),
                    title: Some("Read schema".to_string()),
                    status: Some("pending".to_string()),
                    ..peek_acp::ToolCallUpdate::default()
                }),
                cx,
            );
            view.pump(
                AcpUpdate::ToolCallUpdate(peek_acp::ToolCallUpdate {
                    tool_call_id: "t1".to_string(),
                    status: Some("completed".to_string()),
                    ..peek_acp::ToolCallUpdate::default()
                }),
                cx,
            );
        });
    });

    let rows = cx.update(|cx| view.read(cx).row_count());
    assert_eq!(rows, 1, "the same row, updated");
}

/// A plan replaces itself rather than accumulating, so the node shows the current plan and not
/// every revision of it.
#[gpui_kit::test]
fn a_plan_replaces_itself(cx: &mut TestAppContext) {
    let (_handle, workspace) = open(cx, true);
    let view = view(cx, &workspace);

    let entry = |content: &str, status: &str| peek_document::PlanEntry {
        content: content.to_string(),
        priority: "medium".to_string(),
        status: status.to_string(),
    };

    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.pump(AcpUpdate::Plan(vec![entry("read schema", "pending")]), cx);
            view.pump(AcpUpdate::Plan(vec![entry("read schema", "completed")]), cx);
        });
    });

    let rows = cx.update(|cx| view.read(cx).row_count());
    assert_eq!(rows, 1);
}

/// Forking carries the conversation into a sibling node and leaves the original alone.
#[gpui_kit::test]
fn forking_places_a_sibling_carrying_the_conversation(cx: &mut TestAppContext) {
    let (handle, workspace) = with_messages(cx, vec![message("user", "hi")]);

    assert!(
        cx.update(|cx| workspace
            .read(cx)
            .document(cx)
            .read(cx)
            .selected()
            .is_empty()),
        "nothing is selected, so the button has to select its own node"
    );
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("agent_aaaaaaaa-fork", cx);
        window.render_frame(cx);
    })
    .unwrap();

    let (count, selected) = cx.update(|cx| {
        let document = workspace.read(cx).document(cx);
        let document = document.read(cx);
        (
            document.nodes().len(),
            document.selected().iter().next().cloned(),
        )
    });
    assert_eq!(count, 2, "the fork was placed");
    let forked = selected.expect("the fork is selected");
    assert_ne!(forked, agent_node());
    assert_eq!(
        committed(cx, &workspace).len(),
        1,
        "the source is untouched"
    );
}
