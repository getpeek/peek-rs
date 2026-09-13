//! The agent node's conversation transcript.
//!
//! The shape is frozen by `~/labs/peek/src/canvas/hooks/useExecutePrompt.ts`: a flat struct
//! with a discriminator string, not a tagged enum. Every other field is optional and sits at
//! the same level, so a kind that carries no tool call simply omits those keys.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One entry in an agent node's transcript.
///
/// `kind` and `context_kind` are free strings rather than enums on purpose: real documents
/// carry `contextKind: "schema"`, a value the TypeScript type no longer admits, and an
/// unknown `type` from a newer build must survive a round trip untouched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    /// `Date.now()` milliseconds, as the TypeScript app writes them.
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// For `acp_tool` this is the ACP tool *title* ("Load skill: peek"), not an identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_entries: Option<Vec<PlanEntry>>,
}

impl AgentMessage {
    /// A plain message of `kind` stamped now.
    #[must_use]
    pub fn new(kind: &str, message: impl Into<String>, timestamp: i64) -> Self {
        Self {
            kind: kind.to_string(),
            message: message.into(),
            timestamp,
            context_key: None,
            context_kind: None,
            tool_calls: None,
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            tool_kind: None,
            tool_status: None,
            plan_entries: None,
        }
    }

    #[must_use]
    pub fn is(&self, kind: &str) -> bool {
        self.kind == kind
    }
}

/// A tool the Ollama loop asked for. Retained so those transcripts keep rendering; the ACP
/// path reports its tools as `acp_tool` messages instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(json: &str) -> String {
        let message: AgentMessage = serde_json::from_str(json).expect("parses");
        serde_json::to_string(&message).expect("serializes")
    }

    #[test]
    fn a_plain_message_writes_only_the_three_required_keys() {
        let json = r#"{"type":"user","message":"hi","timestamp":1789117401848}"#;
        assert_eq!(round_trip(json), json);
    }

    #[test]
    fn an_acp_tool_round_trips_in_camel_case() {
        let json = r#"{"type":"acp_tool","message":"","timestamp":1789117401848,"toolCallId":"toolu_01","toolName":"Load skill: peek","toolKind":"other","toolStatus":"completed"}"#;
        let message: AgentMessage = serde_json::from_str(json).expect("parses");
        assert_eq!(message.tool_name.as_deref(), Some("Load skill: peek"));
        assert_eq!(message.tool_status.as_deref(), Some("completed"));
        assert_eq!(round_trip(json), json);
    }

    #[test]
    fn a_legacy_context_kind_survives() {
        let json = r#"{"type":"context","message":"rows","timestamp":1,"contextKey":"abc","contextKind":"schema"}"#;
        let message: AgentMessage = serde_json::from_str(json).expect("parses");
        assert_eq!(message.context_kind.as_deref(), Some("schema"));
        assert_eq!(round_trip(json), json);
    }

    #[test]
    fn an_unknown_message_type_survives_a_round_trip() {
        let json = r#"{"type":"telepathy","message":"soon","timestamp":7}"#;
        assert_eq!(round_trip(json), json);
    }

    #[test]
    fn a_tool_call_keeps_its_arbitrary_arguments() {
        let json = r#"{"type":"tool_call","message":"","timestamp":1,"toolCalls":[{"id":"u1","name":"create_query_node","args":{"query":"select 1"}}]}"#;
        assert_eq!(round_trip(json), json);
    }

    #[test]
    fn a_plan_round_trips_its_entries() {
        let json = r#"{"type":"plan","message":"","timestamp":1,"planEntries":[{"content":"Look","priority":"high","status":"in_progress"}]}"#;
        assert_eq!(round_trip(json), json);
    }
}
