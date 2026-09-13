//! What the agent pushes at the UI, and the channel it arrives on.
//!
//! One connection multiplexes every session, so each event carries the ACP session id and
//! the UI fans out from there. The `serde_json::Value` an ACP notification arrives as is
//! parsed on this side of the channel, so the UI crate needs no JSON dependency.

use tokio::sync::mpsc::UnboundedReceiver;

use crate::update::AcpUpdate;

/// Identifies one outstanding permission prompt. Allocated per process, not per session:
/// the answer travels back on its own path and needs no node scoping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PermissionId(pub(crate) u64);

/// One choice the agent offered for a permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once`, `reject_always`, … — drives which
    /// button variant the prompt renders.
    pub kind: String,
}

/// The agent is asking to run a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    /// The tool's human title. Defaults to "Run a tool" when the agent omits it.
    pub tool_title: String,
    pub tool_kind: Option<String>,
    pub options: Vec<PermissionOption>,
}

/// Everything the UI hears from the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Update {
        session_id: String,
        update: AcpUpdate,
    },
    Permission {
        session_id: String,
        id: PermissionId,
        request: PermissionRequest,
    },
}

/// The receiving end of the event channel.
///
/// A `tokio::sync::mpsc` receiver is a plain future with no reactor of its own, so gpui's
/// executor can await it like any other — the same property that lets `peek_db::Pending`
/// cross the same boundary. The newtype keeps the tokio type out of the UI crate.
#[derive(Debug)]
pub struct AgentEvents(pub(crate) UnboundedReceiver<AgentEvent>);

impl AgentEvents {
    /// The next event, or `None` once the session is dropped.
    pub async fn next(&mut self) -> Option<AgentEvent> {
        self.0.recv().await
    }
}

impl PermissionRequest {
    /// Parses a `RequestPermissionRequest`. Best-effort, like the reference's
    /// `PermissionPrompt`: a shape we don't recognise still renders a cancel button.
    pub(crate) fn from_value(request: &serde_json::Value) -> Self {
        let tool_call = request.get("toolCall");
        let string = |value: Option<&serde_json::Value>, key: &str| {
            value
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        let options = request
            .get("options")
            .and_then(serde_json::Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| {
                        Some(PermissionOption {
                            id: string(Some(option), "optionId")?,
                            name: string(Some(option), "name")?,
                            kind: string(Some(option), "kind").unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            tool_title: string(tool_call, "title").unwrap_or_else(|| "Run a tool".to_string()),
            tool_kind: string(tool_call, "kind"),
            options,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_request_reads_its_title_and_options() {
        let request = PermissionRequest::from_value(&json!({
            "toolCall": { "title": "Edit main.rs", "kind": "edit" },
            "options": [
                { "optionId": "a", "name": "Allow once", "kind": "allow_once" },
                { "optionId": "r", "name": "Reject", "kind": "reject_once" },
            ],
        }));
        assert_eq!(request.tool_title, "Edit main.rs");
        assert_eq!(request.tool_kind.as_deref(), Some("edit"));
        assert_eq!(request.options.len(), 2);
        assert_eq!(request.options[0].id, "a");
        assert_eq!(request.options[1].kind, "reject_once");
    }

    #[test]
    fn a_request_with_no_tool_call_gets_the_default_title() {
        let request = PermissionRequest::from_value(&json!({}));
        assert_eq!(request.tool_title, "Run a tool");
        assert!(request.options.is_empty());
    }

    #[test]
    fn an_option_missing_its_id_is_dropped_rather_than_rendered() {
        let request = PermissionRequest::from_value(&json!({
            "options": [{ "name": "Nameless" }, { "optionId": "a", "name": "Allow" }],
        }));
        assert_eq!(request.options.len(), 1);
        assert_eq!(request.options[0].id, "a");
        // A kind the agent omitted is empty, not a guess.
        assert_eq!(request.options[0].kind, "");
    }
}
