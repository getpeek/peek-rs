//! `session/update` payloads, parsed into the handful of shapes Peek consumes.
//!
//! A port of `~/labs/peek/src/canvas/nodes/Agent/acpUpdates.ts`, and deliberately
//! best-effort in the same way: an unknown or malformed shape degrades to empty text
//! rather than failing the turn. Parsing happens here, on the tokio side, so the
//! `serde_json::Value` never reaches the UI.

use serde_json::Value;

use crate::PlanEntry;

/// The updates the agent node reacts to. Everything else the agent sends is dropped,
/// which is what the reference's `default: break` does.
#[derive(Debug, Clone, PartialEq)]
pub enum AcpUpdate {
    /// A chunk of the agent's answer.
    MessageChunk(String),
    /// A chunk of the agent's reasoning.
    ThoughtChunk(String),
    ToolCall(ToolCallUpdate),
    ToolCallUpdate(ToolCallUpdate),
    Plan(Vec<PlanEntry>),
    ModeChanged(String),
}

/// A `tool_call` or `tool_call_update`. The two carry the same fields; only
/// `tool_call` creates a row, and only `tool_call_update` treats absent fields as
/// "leave what is there alone".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolCallUpdate {
    pub tool_call_id: String,
    /// The ACP tool *title* ("Load skill: peek"), which is what the transcript stores
    /// as `toolName`. `None` on an update that does not restate it.
    pub title: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    /// Text pulled out of the `content` array. Diff and terminal blocks are skipped,
    /// matching the reference.
    pub content: String,
}

impl AcpUpdate {
    /// Parses one `session/update`. `None` for a kind Peek ignores.
    #[must_use]
    pub fn from_value(update: &Value) -> Option<Self> {
        match update.get("sessionUpdate").and_then(Value::as_str)? {
            "agent_message_chunk" => Some(Self::MessageChunk(chunk_text(update))),
            "agent_thought_chunk" => Some(Self::ThoughtChunk(chunk_text(update))),
            "tool_call" => Some(Self::ToolCall(tool_call(update))),
            "tool_call_update" => Some(Self::ToolCallUpdate(tool_call(update))),
            "plan" => Some(Self::Plan(plan_entries(update))),
            "current_mode_update" => update
                .get("currentModeId")
                .and_then(Value::as_str)
                .map(|mode| Self::ModeChanged(mode.to_string())),
            _ => None,
        }
    }
}

/// A content block contributes its text only when it really is a text block.
fn content_block_text(block: Option<&Value>) -> &str {
    let Some(block) = block else { return "" };
    if block.get("type").and_then(Value::as_str) != Some("text") {
        return "";
    }
    block.get("text").and_then(Value::as_str).unwrap_or("")
}

fn chunk_text(update: &Value) -> String {
    content_block_text(update.get("content")).to_string()
}

/// Best-effort text from a tool call's `content` array. Items that are not `content`
/// (diffs, terminals) contribute nothing — Peek renders neither.
fn tool_content_text(content: Option<&Value>) -> String {
    let Some(items) = content.and_then(Value::as_array) else {
        return String::new();
    };
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("content"))
        .map(|item| content_block_text(item.get("content")))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_call(update: &Value) -> ToolCallUpdate {
    let string = |key: &str| update.get(key).and_then(Value::as_str).map(str::to_string);
    ToolCallUpdate {
        tool_call_id: string("toolCallId").unwrap_or_default(),
        title: string("title"),
        kind: string("kind"),
        status: string("status"),
        content: tool_content_text(update.get("content")),
    }
}

fn plan_entries(update: &Value) -> Vec<PlanEntry> {
    update
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: &Value) -> Option<AcpUpdate> {
        AcpUpdate::from_value(value)
    }

    #[test]
    fn an_unknown_session_update_is_ignored() {
        assert!(parse(&json!({ "sessionUpdate": "telepathy" })).is_none());
        assert!(parse(&json!({ "nothing": true })).is_none());
    }

    #[test]
    fn a_chunk_reads_its_text_block() {
        let update = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hello" },
        });
        assert_eq!(
            parse(&update),
            Some(AcpUpdate::MessageChunk("hello".into()))
        );
    }

    #[test]
    fn a_chunk_with_a_non_text_block_reads_as_empty() {
        let update = json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "image", "data": "…" },
        });
        assert_eq!(parse(&update), Some(AcpUpdate::ThoughtChunk(String::new())));
    }

    #[test]
    fn a_chunk_with_no_content_reads_as_empty() {
        let update = json!({ "sessionUpdate": "agent_message_chunk" });
        assert_eq!(parse(&update), Some(AcpUpdate::MessageChunk(String::new())));
    }

    #[test]
    fn tool_content_text_joins_only_content_items() {
        let update = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "toolu_01",
            "title": "Read file",
            "kind": "read",
            "status": "pending",
            "content": [
                { "type": "content", "content": { "type": "text", "text": "first" } },
                { "type": "diff", "path": "/tmp/x" },
                { "type": "content", "content": { "type": "text", "text": "second" } },
                { "type": "content", "content": { "type": "image" } },
            ],
        });
        let Some(AcpUpdate::ToolCall(call)) = parse(&update) else {
            panic!("expected a tool call")
        };
        assert_eq!(call.content, "first\nsecond");
        assert_eq!(call.tool_call_id, "toolu_01");
        assert_eq!(call.title.as_deref(), Some("Read file"));
        assert_eq!(call.kind.as_deref(), Some("read"));
        assert_eq!(call.status.as_deref(), Some("pending"));
    }

    #[test]
    fn a_tool_call_update_may_restate_nothing() {
        let update = json!({ "sessionUpdate": "tool_call_update", "toolCallId": "toolu_01" });
        let Some(AcpUpdate::ToolCallUpdate(call)) = parse(&update) else {
            panic!("expected a tool call update")
        };
        assert_eq!(call.tool_call_id, "toolu_01");
        assert!(call.title.is_none() && call.kind.is_none() && call.status.is_none());
        assert!(call.content.is_empty());
    }

    #[test]
    fn a_plan_parses_its_entries_and_skips_malformed_ones() {
        let update = json!({
            "sessionUpdate": "plan",
            "entries": [
                { "content": "Look", "priority": "high", "status": "pending" },
                { "content": "no priority" },
            ],
        });
        let Some(AcpUpdate::Plan(entries)) = parse(&update) else {
            panic!("expected a plan")
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "Look");
    }

    #[test]
    fn a_plan_with_no_entries_array_is_empty() {
        let update = json!({ "sessionUpdate": "plan" });
        assert_eq!(parse(&update), Some(AcpUpdate::Plan(Vec::new())));
    }

    #[test]
    fn a_mode_update_without_a_mode_id_is_ignored() {
        assert!(parse(&json!({ "sessionUpdate": "current_mode_update" })).is_none());
        let update = json!({ "sessionUpdate": "current_mode_update", "currentModeId": "plan" });
        assert_eq!(parse(&update), Some(AcpUpdate::ModeChanged("plan".into())));
    }
}
