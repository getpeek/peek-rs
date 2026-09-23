//! The local-model turn: prompt, run whatever tools it asks for, repeat until it answers.
//!
//! Ported from `~/labs/peek/src/canvas/nodes/Agent/runAgentConversation.ts`, with one change
//! that matters: the tools are executed by `peek_canvas::tools`, the same code the MCP bridge
//! serves, rather than a second implementation of the same twenty-one operations.

use gpui_kit::{Context, Window};
use peek_canvas::tools::{ToolCall as CanvasCall, agent_params};
use peek_document::{AgentMessage, ToolCall};
use peek_ollama::{Chat, ChatDelta, OllamaSession, ToolCall as ModelCall};
use serde_json::Value;

use super::backend::Agents;
use super::view::{AgentView, now_ms};

/// How many times the model may be asked in one turn. Bounds round trips, not tool calls.
const MAX_TOOL_ITERATIONS: usize = 8;

/// Injected after each round of tools, so the model uses the results rather than looping. It
/// never reaches the document: the user did not say this.
const POST_TOOL_INSTRUCTION: &str = "Continue using tools as needed to finish the user's request, \
then summarize what you did. If the tool results already answer the request, respond to the user \
instead of calling more tools.";

/// Runs one turn to completion.
pub(super) fn ask(
    view: &mut AgentView,
    question: String,
    window: &mut Window,
    cx: &mut Context<AgentView>,
) {
    let Some(session) = Agents::ollama(cx) else {
        view.stage(
            AgentMessage::new(
                "system",
                "No Ollama backend configured. Add an `ai.ollama` block to ~/peek/settings.json."
                    .to_string(),
                now_ms(),
            ),
            cx,
        );
        view.finish_turn(cx);
        return;
    };

    // Read *before* the user's turn is staged, so the working set is prior + question exactly
    // once — the reference is explicit about this ordering for the same reason.
    let engine = crate::database::Database::engine(cx);
    let mut history = view.committed_messages(cx);
    let node = view.node().clone();
    let document = view.document_handle();

    // Rows the user ran, for a model that cannot run anything. Staged before the question so
    // the transcript reads in the order the model receives it.
    let fresh = super::context::gather(document.read(cx), &node, &history);
    for message in fresh {
        history.push(message.clone());
        view.stage(message, cx);
    }

    let task = cx.spawn_in(window, async move |this, cx| {
        let mut working = Chat::new()
            .tools(super::tools::all())
            .system(super::tools::system_prompt(engine));
        for message in &history {
            working = replay(working, message);
        }
        working = working.user(question);

        for iteration in 0..MAX_TOOL_ITERATIONS {
            let Ok(Some(round)) = run_round(&session, working.clone(), &this, cx).await else {
                return;
            };
            let Some(calls) = round.calls else {
                return;
            };

            // The whole batch, sorted, is the signature: a model that asks for the same things
            // twice is stuck, and looping until the cap wastes the user's time and tokens.
            if iteration + 1 == MAX_TOOL_ITERATIONS {
                let _ = this.update(cx, |view, cx| {
                    view.stage(
                        AgentMessage::new(
                            "system",
                            format!("Tool iteration limit ({MAX_TOOL_ITERATIONS}) reached."),
                            now_ms(),
                        ),
                        cx,
                    );
                    view.finish_turn(cx);
                });
                return;
            }

            working = working.assistant_calls(round.answer.clone(), &calls);
            let mut results = Vec::with_capacity(calls.len());
            for call in &calls {
                let Ok(result) = cx.update(|_, cx| {
                    document.update(cx, |document, cx| {
                        let params = agent_params(
                            document,
                            &node,
                            CanvasCall {
                                method: &call.name,
                                params: &call.args,
                            },
                        );
                        let outcome = peek_canvas::tools::execute(
                            document,
                            CanvasCall {
                                method: &call.name,
                                params: &params,
                            },
                        );
                        cx.notify();
                        outcome.reply
                    })
                }) else {
                    return;
                };
                results.push(describe(&result));
            }

            let Ok(()) = this.update(cx, |view, cx| {
                record(view, &round.answer, &calls, &results, cx);
            }) else {
                return;
            };

            for result in &results {
                working = working.tool_result(result.text.clone());
            }
            working = working.user(POST_TOOL_INSTRUCTION);
        }
    });
    view.hold_prompt(task);
}

struct Round {
    answer: String,
    /// `None` once the model answered instead of asking for tools — the turn is over.
    calls: Option<Vec<ModelCall>>,
}

/// One round trip to the model, streaming its answer into the view as it arrives.
async fn run_round(
    session: &std::sync::Arc<OllamaSession>,
    chat: Chat,
    this: &gpui_kit::WeakEntity<AgentView>,
    cx: &mut gpui_kit::AsyncWindowContext,
) -> Result<Option<Round>, ()> {
    let mut turn = session.chat(chat);
    let mut calls = Vec::new();

    while let Some(delta) = turn.next().await {
        match delta {
            ChatDelta::Chunk(text) => {
                if this
                    .update(cx, |view, cx| view.stream_chunk(&text, cx))
                    .is_err()
                {
                    return Err(());
                }
            }
            ChatDelta::Calls(asked) => calls = asked,
            ChatDelta::Done(answer) => {
                if calls.is_empty() {
                    let _ = this.update(cx, AgentView::finish_turn);
                    return Ok(Some(Round {
                        answer,
                        calls: None,
                    }));
                }
                return Ok(Some(Round {
                    answer,
                    calls: Some(calls),
                }));
            }
            ChatDelta::Failed(error) => {
                let _ = this.update(cx, |view, cx| {
                    view.stage(
                        AgentMessage::new("system", format!("The model failed: {error}"), now_ms()),
                        cx,
                    );
                    view.finish_turn(cx);
                });
                return Ok(None);
            }
        }
    }
    // The stream ended without saying so, which a cancelled turn does.
    let _ = this.update(cx, AgentView::finish_turn);
    Ok(None)
}

struct ToolOutcome {
    text: String,
    is_error: bool,
}

/// The shared `{"error"}` convention, rendered the way a model reads best.
fn describe(reply: &Value) -> ToolOutcome {
    if let Some(message) = reply.get("error").and_then(Value::as_str) {
        return ToolOutcome {
            text: format!("Error: {message}"),
            is_error: true,
        };
    }
    ToolOutcome {
        text: reply.to_string(),
        is_error: false,
    }
}

/// Writes the round into the transcript: the call, then one result per call.
fn record(
    view: &mut AgentView,
    answer: &str,
    calls: &[ModelCall],
    results: &[ToolOutcome],
    cx: &mut Context<AgentView>,
) {
    let mut message = AgentMessage::new("tool_call", answer.to_string(), now_ms());
    message.tool_calls = Some(
        calls
            .iter()
            .map(|call| ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                args: call.args.clone(),
            })
            .collect(),
    );
    view.stage(message, cx);

    for (call, result) in calls.iter().zip(results) {
        let mut message = AgentMessage::new("tool_result", result.text.clone(), now_ms());
        message.tool_call_id = Some(call.id.clone());
        message.tool_name = Some(call.name.clone());
        message.is_error = result.is_error.then_some(true);
        view.stage(message, cx);
    }
}

/// Replays a persisted message into the working set the model sees.
///
/// `context` becomes the reference's question-and-acknowledgement pair, which is how rows the
/// user ran reach a model that cannot run anything itself.
fn replay(chat: Chat, message: &AgentMessage) -> Chat {
    match message.kind.as_str() {
        "user" => chat.user(message.message.clone()),
        "assistant" => chat.assistant(message.message.clone()),
        "system" => chat.system(message.message.clone()),
        "context" if message.context_kind.as_deref() == Some(super::context::QUERY_CONTEXT) => chat
            .user(message.message.clone())
            .assistant("Ok! I know which query node you mean."),
        "context" => chat
            .user(format!(
                "Here is a fresh query and data:\n{}",
                message.message
            ))
            .assistant("Ok! I've received the new query and data"),
        "tool_call" => {
            let calls: Vec<ModelCall> = message
                .tool_calls
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|call| ModelCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    args: call.args.clone(),
                })
                .collect();
            chat.assistant_calls(message.message.clone(), &calls)
        }
        "tool_result" => chat.tool_result(message.message.clone()),
        // `thought`, `plan` and `acp_tool` belong to the other backend and have no place here.
        _ => chat,
    }
}

#[cfg(test)]
mod tests {
    use super::{POST_TOOL_INSTRUCTION, describe, replay};
    use peek_document::{AgentMessage, ToolCall};
    use peek_ollama::Chat;
    use serde_json::json;

    fn message(kind: &str, text: &str) -> AgentMessage {
        AgentMessage::new(kind, text.to_string(), 1)
    }

    fn replay_all(messages: &[AgentMessage]) -> Chat {
        messages.iter().fold(Chat::new(), replay)
    }

    /// A `context` message is rows the user ran, and the model has to be told about them as a
    /// question it already answered — otherwise it reads as an instruction it should act on.
    #[test]
    fn a_context_message_becomes_a_question_and_an_acknowledgement() {
        let chat = replay_all(&[message("context", "Query: select 1")]);
        assert_eq!(chat.len(), 2);
    }

    /// The transcript carries kinds the other backend produced. Replaying a `thought` as if the
    /// model had said it would put words in its mouth.
    #[test]
    fn acp_only_kinds_are_left_out_of_the_model_context() {
        let chat = replay_all(&[
            message("thought", "hmm"),
            message("plan", ""),
            message("acp_tool", "ran something"),
            message("user", "hi"),
        ]);
        assert_eq!(chat.len(), 1, "only the user turn survives");
    }

    #[test]
    fn a_tool_round_replays_as_a_call_then_a_result() {
        let mut call = message("tool_call", "let me look");
        call.tool_calls = Some(vec![ToolCall {
            id: "call_1".to_string(),
            name: "get_pages".to_string(),
            args: json!({}),
        }]);
        let chat = replay_all(&[message("user", "hi"), call, message("tool_result", "[]")]);

        assert_eq!(chat.len(), 3);
    }

    #[test]
    fn the_shared_error_convention_is_rendered_for_the_model() {
        let failed = describe(&json!({ "error": "node text_1 not found" }));
        assert_eq!(failed.text, "Error: node text_1 not found");
        assert!(failed.is_error);

        let ok = describe(&json!({ "nodeId": "query_1" }));
        assert!(!ok.is_error);
        assert!(ok.text.contains("query_1"));
    }

    /// The nudge steers the model but was never something the user said, so it must not reach
    /// the document — only the working set the model sees.
    #[test]
    fn the_post_tool_nudge_tells_the_model_to_use_what_it_got() {
        assert!(POST_TOOL_INSTRUCTION.contains("already answer the request"));
    }
}
