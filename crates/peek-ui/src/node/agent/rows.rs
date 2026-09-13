//! Turning a flat transcript into the blocks that are actually drawn.
//!
//! Three of the message kinds do not render one-to-one, so the list has to be folded before it
//! can be measured — `~/labs/peek/src/canvas/nodes/Agent/MessageList.tsx` does the same pre-pass:
//!
//! - a `tool_call` swallows every `tool_result` that answers it, into one disclosure;
//! - a `context` message is labelled by how many came before it ("inserted", then "updated");
//! - the first message, when it is a `system` one, is the seeded prompt and is not shown.
//!
//! Folding is incremental because a turn appends one message at a time; re-folding the whole
//! vector per streamed chunk would be quadratic in the length of the conversation.

use std::collections::HashSet;

use peek_document::AgentMessage;

/// One drawn block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Row {
    /// Renders from one message: user, assistant, context, thought, plan, `acp_tool`, system, or
    /// a `tool_result` whose call never arrived.
    Message {
        message: usize,
        /// The reference's "Context inserted" vs "Context updated".
        context_updated: bool,
    },
    /// A `tool_call` and the results it consumed, as one disclosure.
    ToolPair {
        message: usize,
        blocks: Vec<ToolBlock>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolBlock {
    /// Index into the `tool_calls` array of the owning message.
    pub(super) call: usize,
    pub(super) result: Option<usize>,
    pub(super) is_error: bool,
}

/// The fold's running state, kept so an appended message costs one step rather than a re-fold.
#[derive(Debug, Default)]
pub(super) struct Builder {
    /// Tool-call ids already drawn inside a pair, so their results are not drawn again.
    consumed: HashSet<String>,
    contexts_seen: usize,
    /// Message index -> the row it produced, for the remeasure a patched message needs.
    row_of_message: Vec<Option<usize>>,
    rows: usize,
}

impl Builder {
    /// Folds `message`, which sits at `index` in `all`, and returns the row it produced —
    /// `None` when an earlier `tool_call` already drew it.
    pub(super) fn push(&mut self, index: usize, all: &[AgentMessage]) -> Option<Row> {
        debug_assert_eq!(self.row_of_message.len(), index, "messages fold in order");
        let message = &all[index];
        let row = self.fold(index, message, all);
        self.row_of_message.push(row.as_ref().map(|_| self.rows));
        if row.is_some() {
            self.rows += 1;
        }
        row
    }

    fn fold(&mut self, index: usize, message: &AgentMessage, all: &[AgentMessage]) -> Option<Row> {
        // The seeded system prompt is the model's instructions, not part of the conversation.
        if index == 0 && message.is("system") {
            return None;
        }
        if message.is("tool_result") {
            let answered = message
                .tool_call_id
                .as_ref()
                .is_some_and(|id| self.consumed.contains(id));
            if answered {
                return None;
            }
        }
        if message.is("context") {
            let updated = self.contexts_seen > 0;
            self.contexts_seen += 1;
            return Some(Row::Message {
                message: index,
                context_updated: updated,
            });
        }
        if message.is("tool_call") {
            return Some(self.pair(index, message, all));
        }
        Some(Row::Message {
            message: index,
            context_updated: false,
        })
    }

    /// Claims each of the call's results. The search runs over the whole vector rather than
    /// forwards only, because a re-fold after undo sees them all at once.
    fn pair(&mut self, index: usize, message: &AgentMessage, all: &[AgentMessage]) -> Row {
        let calls = message.tool_calls.as_deref().unwrap_or_default();
        let blocks = calls
            .iter()
            .enumerate()
            .map(|(position, call)| {
                let result = all.iter().position(|candidate| {
                    candidate.is("tool_result")
                        && candidate.tool_call_id.as_deref() == Some(call.id.as_str())
                });
                if let Some(result) = result {
                    self.consumed.insert(call.id.clone());
                    return ToolBlock {
                        call: position,
                        result: Some(result),
                        is_error: all[result].is_error == Some(true),
                    };
                }
                ToolBlock {
                    call: position,
                    result: None,
                    is_error: false,
                }
            })
            .collect();
        Row::ToolPair {
            message: index,
            blocks,
        }
    }

    /// The row a message is drawn in, for the one-row remeasure a patch needs.
    pub(super) fn row_of(&self, message: usize) -> Option<usize> {
        self.row_of_message.get(message).copied().flatten()
    }
}

/// Folds a whole transcript, for the paths that get one all at once: opening a document, undo,
/// and a fork adopting its source's history.
pub(super) fn build(messages: &[AgentMessage]) -> (Builder, Vec<Row>) {
    let mut builder = Builder::default();
    let mut rows = Vec::with_capacity(messages.len());
    for index in 0..messages.len() {
        if let Some(row) = builder.push(index, messages) {
            rows.push(row);
        }
    }
    (builder, rows)
}

#[cfg(test)]
mod tests {
    use super::{Builder, Row, build};
    use peek_document::{AgentMessage, ToolCall};
    use serde_json::json;

    fn message(kind: &str, text: &str) -> AgentMessage {
        AgentMessage::new(kind, text.to_string(), 0)
    }

    fn call(id: &str) -> AgentMessage {
        let mut message = message("tool_call", "");
        message.tool_calls = Some(vec![ToolCall {
            id: id.to_string(),
            name: "get_pages".to_string(),
            args: json!({}),
        }]);
        message
    }

    fn result(id: &str, is_error: bool) -> AgentMessage {
        let mut message = message("tool_result", "[]");
        message.tool_call_id = Some(id.to_string());
        message.is_error = is_error.then_some(true);
        message
    }

    #[test]
    fn a_result_its_call_claimed_is_not_drawn_twice() {
        let messages = vec![message("user", "hi"), call("t1"), result("t1", false)];
        let (_, rows) = build(&messages);

        assert_eq!(rows.len(), 2, "the result folded into the call");
        let Row::ToolPair { blocks, .. } = &rows[1] else {
            panic!("expected a pair, got {:?}", rows[1]);
        };
        assert_eq!(blocks[0].result, Some(2));
    }

    /// A result with no call is a transcript from a turn that was cut short; drawing nothing
    /// would silently lose it.
    #[test]
    fn an_orphan_result_is_drawn_on_its_own() {
        let messages = vec![message("user", "hi"), result("nobody", false)];
        let (_, rows) = build(&messages);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[1], Row::Message { message: 1, .. }));
    }

    #[test]
    fn a_failed_result_marks_its_block() {
        let messages = vec![call("t1"), result("t1", true)];
        let (_, rows) = build(&messages);
        let Row::ToolPair { blocks, .. } = &rows[0] else {
            panic!("expected a pair");
        };
        assert!(blocks[0].is_error);
    }

    #[test]
    fn the_first_context_is_inserted_and_later_ones_are_updates() {
        let messages = vec![
            message("context", "rows"),
            message("user", "hi"),
            message("context", "more rows"),
        ];
        let (_, rows) = build(&messages);

        assert_eq!(
            rows[0],
            Row::Message {
                message: 0,
                context_updated: false
            }
        );
        assert_eq!(
            rows[2],
            Row::Message {
                message: 2,
                context_updated: true
            }
        );
    }

    /// The seeded prompt is instructions, not conversation — but a system message the agent
    /// emits mid-turn (a failed start, a stop reason) has to be visible.
    #[test]
    fn only_a_leading_system_message_is_hidden() {
        let leading = vec![message("system", "you are"), message("user", "hi")];
        let (_, rows) = build(&leading);
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0], Row::Message { message: 1, .. }));

        let later = vec![message("user", "hi"), message("system", "turn failed")];
        let (_, rows) = build(&later);
        assert_eq!(rows.len(), 2);
    }

    /// The property the streaming path depends on: appending one message at a time has to give
    /// the same rows as folding the finished vector, or a turn renders differently from the
    /// document it just wrote.
    #[test]
    fn folding_incrementally_matches_folding_all_at_once() {
        let messages = vec![
            message("system", "you are"),
            message("user", "hi"),
            message("context", "rows"),
            call("t1"),
            result("t1", false),
            message("assistant", "done"),
            message("context", "more"),
        ];

        let mut builder = Builder::default();
        let mut incremental = Vec::new();
        for index in 0..messages.len() {
            if let Some(row) = builder.push(index, &messages) {
                incremental.push(row);
            }
        }

        let (_, whole) = build(&messages);
        assert_eq!(incremental, whole);
    }

    #[test]
    fn a_message_knows_the_row_it_landed_in() {
        let messages = vec![
            message("system", "you are"),
            message("user", "hi"),
            call("t1"),
        ];
        let (builder, _) = build(&messages);

        assert_eq!(builder.row_of(0), None, "the hidden prompt has no row");
        assert_eq!(builder.row_of(1), Some(0));
        assert_eq!(builder.row_of(2), Some(1));
    }
}
