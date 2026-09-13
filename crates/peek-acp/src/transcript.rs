//! Turns a stream of [`AcpUpdate`]s into the node's persisted [`AgentMessage`] list.
//!
//! A port of `~/labs/peek/src/canvas/nodes/Agent/useAcpMessageSink.ts`. Streamed text
//! accumulates in a buffer until a boundary — a tool call, a switch between answer and
//! thought, or turn end — and only then becomes a message. The buffers live here rather
//! than in the view so the whole translation is testable without a window.
//!
//! Unlike the reference, nothing is written to the document as it arrives: the caller
//! keeps the [`Transcript`] for the length of a turn and commits `messages` once.

use peek_document::{AgentMessage, PlanEntry};

use crate::update::{AcpUpdate, ToolCallUpdate};

/// Which live row the transcript is currently streaming into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    Answer,
    Thought,
}

/// What one update changed, so the caller can tell a virtualized list what to do
/// without diffing the whole message vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Change {
    /// Messages appended to the end.
    pub appended: usize,
    /// Index of a message patched in place.
    pub patched: Option<usize>,
    /// The live preview text grew; its kind is [`Transcript::preview`].
    pub streamed: bool,
    /// The preview appeared, vanished, or swapped kind.
    pub preview_changed: bool,
    /// A `current_mode_update`; the transcript itself is unchanged.
    pub mode: bool,
}

impl Change {
    fn appended(count: usize) -> Self {
        Self {
            appended: count,
            ..Self::default()
        }
    }
}

/// The staged messages of one turn plus the in-flight streaming buffers.
#[derive(Debug, Default)]
pub struct Transcript {
    messages: Vec<AgentMessage>,
    answer: String,
    thought: String,
    preview: Option<Preview>,
    mode: Option<String>,
}

impl Transcript {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn messages(&self) -> &[AgentMessage] {
        &self.messages
    }

    /// The text of the live row, if one is showing.
    #[must_use]
    pub fn preview_text(&self) -> &str {
        match self.preview {
            Some(Preview::Answer) => &self.answer,
            Some(Preview::Thought) => &self.thought,
            None => "",
        }
    }

    #[must_use]
    pub fn preview(&self) -> Option<Preview> {
        self.preview
    }

    /// The mode the agent last reported, which arrives out of band from the prompt.
    #[must_use]
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

    /// Appends a message the app authored rather than the agent — the user's prompt, or
    /// a `system` note when a turn fails to start.
    pub fn push(&mut self, message: AgentMessage) -> Change {
        self.messages.push(message);
        Change::appended(1)
    }

    /// Takes the staged messages, leaving the transcript empty for the next turn.
    pub fn take(&mut self) -> Vec<AgentMessage> {
        std::mem::take(&mut self.messages)
    }

    /// Applies one update. `now` is the timestamp to stamp any message this creates,
    /// passed in so tests are deterministic.
    pub fn apply(&mut self, update: AcpUpdate, now: i64) -> Change {
        match update {
            AcpUpdate::MessageChunk(text) => self.stream(Preview::Answer, &text, now),
            AcpUpdate::ThoughtChunk(text) => self.stream(Preview::Thought, &text, now),
            AcpUpdate::ToolCall(call) => {
                let mut change = self.flush_all(now);
                self.messages.push(tool_message(&call, now));
                change.appended += 1;
                change
            }
            AcpUpdate::ToolCallUpdate(call) => self.patch_tool(&call),
            AcpUpdate::Plan(entries) => self.upsert_plan(entries, now),
            AcpUpdate::ModeChanged(mode) => {
                self.mode = Some(mode);
                Change {
                    mode: true,
                    ..Change::default()
                }
            }
        }
    }

    /// Ends the turn: whatever is still buffered becomes a message. Called on the stop
    /// reason, on an error, and when the user stops the turn.
    pub fn finish(&mut self, now: i64) -> Change {
        self.flush_all(now)
    }

    /// A chunk of streamed text. Switching kind flushes the other buffer first, which is
    /// what makes "thought, then answer, then thought" three separate messages.
    fn stream(&mut self, kind: Preview, text: &str, now: i64) -> Change {
        let mut change = match kind {
            Preview::Answer => self.flush(Preview::Thought, now),
            Preview::Thought => self.flush(Preview::Answer, now),
        };
        let previous = self.preview;
        match kind {
            Preview::Answer => self.answer.push_str(text),
            Preview::Thought => self.thought.push_str(text),
        }
        self.preview = Some(kind);
        change.streamed = true;
        change.preview_changed |= previous != self.preview;
        change
    }

    /// Thought first, then answer — the order `AgentView.tsx` renders them in, so a turn
    /// that ends mid-thought reads in the order it happened.
    fn flush_all(&mut self, now: i64) -> Change {
        let thought = self.flush(Preview::Thought, now);
        let answer = self.flush(Preview::Answer, now);
        Change {
            appended: thought.appended + answer.appended,
            patched: None,
            streamed: false,
            preview_changed: thought.preview_changed || answer.preview_changed,
            mode: false,
        }
    }

    /// A buffer becomes a message only if it holds more than whitespace; either way the
    /// buffer and its preview are cleared.
    fn flush(&mut self, kind: Preview, now: i64) -> Change {
        let (buffer, message_kind) = match kind {
            Preview::Answer => (&mut self.answer, "assistant"),
            Preview::Thought => (&mut self.thought, "thought"),
        };
        let text = std::mem::take(buffer);
        let preview_changed = self.preview == Some(kind);
        if preview_changed {
            self.preview = None;
        }
        if text.trim().is_empty() {
            return Change {
                preview_changed,
                ..Change::default()
            };
        }
        self.messages
            .push(AgentMessage::new(message_kind, text, now));
        Change {
            appended: 1,
            preview_changed,
            ..Change::default()
        }
    }

    /// Patches the `acp_tool` row with this id. An update for an id we never saw is
    /// dropped rather than appended — the reference's `map` does the same.
    fn patch_tool(&mut self, call: &ToolCallUpdate) -> Change {
        let found = self.messages.iter_mut().enumerate().find(|(_, message)| {
            message.is("acp_tool") && message.tool_call_id.as_deref() == Some(&call.tool_call_id)
        });
        let Some((index, message)) = found else {
            return Change::default();
        };
        if let Some(status) = &call.status {
            message.tool_status = Some(status.clone());
        }
        if let Some(title) = &call.title {
            message.tool_name = Some(title.clone());
        }
        if call.status.as_deref() == Some("failed") {
            message.is_error = Some(true);
        }
        if !call.content.is_empty() {
            if message.message.is_empty() {
                message.message.clone_from(&call.content);
            } else {
                message.message.push('\n');
                message.message.push_str(&call.content);
            }
        }
        Change {
            patched: Some(index),
            ..Change::default()
        }
    }

    /// A plan replaces the last plan rather than appending, so a turn shows one plan that
    /// evolves instead of a stack of snapshots.
    fn upsert_plan(&mut self, entries: Vec<PlanEntry>, now: i64) -> Change {
        if let Some((index, last)) = self
            .messages
            .iter_mut()
            .enumerate()
            .rfind(|(_, message)| message.is("plan"))
        {
            last.plan_entries = Some(entries);
            return Change {
                patched: Some(index),
                ..Change::default()
            };
        }
        let mut message = AgentMessage::new("plan", "", now);
        message.plan_entries = Some(entries);
        self.messages.push(message);
        Change::appended(1)
    }
}

fn tool_message(call: &ToolCallUpdate, now: i64) -> AgentMessage {
    let mut message = AgentMessage::new("acp_tool", call.content.clone(), now);
    message.tool_call_id = Some(call.tool_call_id.clone());
    message.tool_name = Some(call.title.clone().unwrap_or_else(|| "tool".to_string()));
    message.tool_kind = Some(call.kind.clone().unwrap_or_else(|| "other".to_string()));
    message.tool_status = Some(call.status.clone().unwrap_or_else(|| "pending".to_string()));
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_789_117_400_000;

    fn chunk(text: &str) -> AcpUpdate {
        AcpUpdate::MessageChunk(text.to_string())
    }

    fn thought(text: &str) -> AcpUpdate {
        AcpUpdate::ThoughtChunk(text.to_string())
    }

    fn call(id: &str, status: &str) -> ToolCallUpdate {
        ToolCallUpdate {
            tool_call_id: id.to_string(),
            title: Some("Read file".to_string()),
            kind: Some("read".to_string()),
            status: Some(status.to_string()),
            content: String::new(),
        }
    }

    fn kinds(transcript: &Transcript) -> Vec<&str> {
        transcript
            .messages()
            .iter()
            .map(|message| message.kind.as_str())
            .collect()
    }

    #[test]
    fn a_message_chunk_flushes_a_pending_thought() {
        let mut transcript = Transcript::new();
        transcript.apply(thought("thinking"), NOW);
        assert_eq!(transcript.preview(), Some(Preview::Thought));

        transcript.apply(chunk("answering"), NOW);
        assert_eq!(kinds(&transcript), ["thought"]);
        assert_eq!(transcript.messages()[0].message, "thinking");
        assert_eq!(transcript.preview(), Some(Preview::Answer));
        assert_eq!(transcript.preview_text(), "answering");
    }

    #[test]
    fn a_thought_chunk_flushes_a_pending_answer() {
        let mut transcript = Transcript::new();
        transcript.apply(chunk("half an answer"), NOW);
        transcript.apply(thought("wait"), NOW);
        assert_eq!(kinds(&transcript), ["assistant"]);
        assert_eq!(transcript.preview(), Some(Preview::Thought));
    }

    #[test]
    fn chunks_of_the_same_kind_accumulate_into_one_message() {
        let mut transcript = Transcript::new();
        transcript.apply(chunk("Hello, "), NOW);
        transcript.apply(chunk("world"), NOW);
        assert!(transcript.messages().is_empty());
        assert_eq!(transcript.preview_text(), "Hello, world");

        transcript.finish(NOW);
        assert_eq!(kinds(&transcript), ["assistant"]);
        assert_eq!(transcript.messages()[0].message, "Hello, world");
        assert_eq!(transcript.preview(), None);
    }

    #[test]
    fn a_tool_call_flushes_both_buffers_then_appends_an_acp_tool() {
        let mut transcript = Transcript::new();
        transcript.apply(thought("hmm"), NOW);
        transcript.apply(chunk("one moment"), NOW);
        let change = transcript.apply(AcpUpdate::ToolCall(call("t1", "pending")), NOW);

        assert_eq!(kinds(&transcript), ["thought", "assistant", "acp_tool"]);
        // The thought was flushed by the earlier answer chunk; this update contributed
        // the flushed answer and the tool row.
        assert_eq!(change.appended, 2);
        let tool = &transcript.messages()[2];
        assert_eq!(tool.tool_call_id.as_deref(), Some("t1"));
        assert_eq!(tool.tool_name.as_deref(), Some("Read file"));
        assert_eq!(tool.tool_status.as_deref(), Some("pending"));
        assert_eq!(transcript.preview(), None);
    }

    #[test]
    fn a_tool_call_without_a_title_or_kind_gets_the_reference_defaults() {
        let mut transcript = Transcript::new();
        transcript.apply(
            AcpUpdate::ToolCall(ToolCallUpdate {
                tool_call_id: "t1".into(),
                ..ToolCallUpdate::default()
            }),
            NOW,
        );
        let tool = &transcript.messages()[0];
        assert_eq!(tool.tool_name.as_deref(), Some("tool"));
        assert_eq!(tool.tool_kind.as_deref(), Some("other"));
        assert_eq!(tool.tool_status.as_deref(), Some("pending"));
    }

    #[test]
    fn a_tool_call_update_patches_the_row_with_the_same_id_in_place() {
        let mut transcript = Transcript::new();
        transcript.apply(AcpUpdate::ToolCall(call("t1", "pending")), NOW);
        transcript.apply(AcpUpdate::ToolCall(call("t2", "pending")), NOW);

        let change = transcript.apply(AcpUpdate::ToolCallUpdate(call("t1", "completed")), NOW);
        assert_eq!(change.patched, Some(0));
        assert_eq!(change.appended, 0);
        assert_eq!(transcript.messages().len(), 2);
        assert_eq!(
            transcript.messages()[0].tool_status.as_deref(),
            Some("completed")
        );
        assert_eq!(
            transcript.messages()[1].tool_status.as_deref(),
            Some("pending")
        );
    }

    #[test]
    fn a_failed_tool_call_update_sets_is_error() {
        let mut transcript = Transcript::new();
        transcript.apply(AcpUpdate::ToolCall(call("t1", "pending")), NOW);
        assert_eq!(transcript.messages()[0].is_error, None);

        transcript.apply(AcpUpdate::ToolCallUpdate(call("t1", "failed")), NOW);
        assert_eq!(transcript.messages()[0].is_error, Some(true));
    }

    #[test]
    fn a_tool_call_update_appends_its_extra_content_on_a_new_line() {
        let mut transcript = Transcript::new();
        let mut opened = call("t1", "in_progress");
        opened.content = "reading".into();
        transcript.apply(AcpUpdate::ToolCall(opened), NOW);

        let mut more = call("t1", "completed");
        more.content = "done".into();
        transcript.apply(AcpUpdate::ToolCallUpdate(more), NOW);
        assert_eq!(transcript.messages()[0].message, "reading\ndone");
    }

    #[test]
    fn extra_content_on_an_empty_tool_row_does_not_lead_with_a_newline() {
        let mut transcript = Transcript::new();
        transcript.apply(AcpUpdate::ToolCall(call("t1", "pending")), NOW);
        let mut more = call("t1", "completed");
        more.content = "output".into();
        transcript.apply(AcpUpdate::ToolCallUpdate(more), NOW);
        assert_eq!(transcript.messages()[0].message, "output");
    }

    #[test]
    fn a_tool_call_update_restating_nothing_leaves_the_row_alone() {
        let mut transcript = Transcript::new();
        transcript.apply(AcpUpdate::ToolCall(call("t1", "in_progress")), NOW);
        let before = transcript.messages()[0].clone();

        transcript.apply(
            AcpUpdate::ToolCallUpdate(ToolCallUpdate {
                tool_call_id: "t1".into(),
                ..ToolCallUpdate::default()
            }),
            NOW,
        );
        assert_eq!(transcript.messages()[0], before);
    }

    #[test]
    fn a_tool_call_update_for_an_unknown_id_changes_nothing() {
        let mut transcript = Transcript::new();
        transcript.apply(AcpUpdate::ToolCall(call("t1", "pending")), NOW);
        let change = transcript.apply(AcpUpdate::ToolCallUpdate(call("ghost", "completed")), NOW);
        assert_eq!(change, Change::default());
        assert_eq!(transcript.messages().len(), 1);
    }

    fn entry(content: &str, status: &str) -> PlanEntry {
        PlanEntry {
            content: content.to_string(),
            priority: "high".to_string(),
            status: status.to_string(),
        }
    }

    #[test]
    fn a_plan_with_no_previous_plan_appends_one() {
        let mut transcript = Transcript::new();
        let change = transcript.apply(AcpUpdate::Plan(vec![entry("Look", "pending")]), NOW);
        assert_eq!(change.appended, 1);
        assert_eq!(kinds(&transcript), ["plan"]);
        assert_eq!(transcript.messages()[0].message, "");
    }

    #[test]
    fn a_plan_update_replaces_the_last_plan_rather_than_appending() {
        let mut transcript = Transcript::new();
        transcript.apply(AcpUpdate::Plan(vec![entry("Look", "pending")]), NOW);
        transcript.apply(AcpUpdate::ToolCall(call("t1", "pending")), NOW);

        let change = transcript.apply(
            AcpUpdate::Plan(vec![entry("Look", "completed"), entry("Size", "pending")]),
            NOW,
        );
        assert_eq!(change.patched, Some(0));
        assert_eq!(kinds(&transcript), ["plan", "acp_tool"]);
        let entries = transcript.messages()[0].plan_entries.as_ref().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, "completed");
    }

    #[test]
    fn a_whitespace_only_buffer_is_dropped_on_flush() {
        let mut transcript = Transcript::new();
        transcript.apply(chunk("   \n  "), NOW);
        let change = transcript.finish(NOW);
        assert!(transcript.messages().is_empty());
        assert_eq!(change.appended, 0);
        assert!(change.preview_changed);
    }

    #[test]
    fn a_current_mode_update_changes_no_messages() {
        let mut transcript = Transcript::new();
        transcript.apply(chunk("text"), NOW);
        let change = transcript.apply(AcpUpdate::ModeChanged("plan".into()), NOW);
        assert!(change.mode);
        assert_eq!(change.appended, 0);
        assert_eq!(transcript.mode(), Some("plan"));
        assert_eq!(transcript.preview_text(), "text");
    }

    #[test]
    fn finishing_twice_appends_nothing_the_second_time() {
        let mut transcript = Transcript::new();
        transcript.apply(chunk("done"), NOW);
        assert_eq!(transcript.finish(NOW).appended, 1);
        assert_eq!(transcript.finish(NOW).appended, 0);
        assert_eq!(transcript.messages().len(), 1);
    }

    #[test]
    fn taking_the_turn_leaves_the_transcript_ready_for_the_next_one() {
        let mut transcript = Transcript::new();
        transcript.push(AgentMessage::new("user", "hi", NOW));
        transcript.apply(chunk("hello"), NOW);
        transcript.finish(NOW);

        let taken = transcript.take();
        assert_eq!(taken.len(), 2);
        assert!(transcript.messages().is_empty());
        assert_eq!(transcript.preview(), None);
    }
}
