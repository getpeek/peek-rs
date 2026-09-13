//! The [`FrontendBridge`] the tools call, and the request channel it feeds.
//!
//! The Tauri app round-tripped every tool call through a webview event and a
//! `mcp_respond` command. Here the "frontend" is the gpui main thread that owns the
//! document, so the round trip is a channel: this side parks a reply sender and awaits it,
//! the UI drains the channel, mutates the document on the main thread, and answers.
//!
//! The five-second deadline stays on this side deliberately — a busy or stalled UI then
//! fails one tool call with the message the agent already understands, rather than wedging
//! the agent's turn.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::bridge::FrontendBridge;

/// How long a tool call waits for the UI before giving up. The reference's timeout.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// One tool call waiting for the document to answer it.
///
/// Answering is a move, so a request can be answered exactly once and a dropped request
/// releases its waiter rather than stranding the agent.
#[derive(Debug)]
pub struct McpRequest {
    method: String,
    params: Value,
    reply: oneshot::Sender<Value>,
}

impl McpRequest {
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    #[must_use]
    pub fn params(&self) -> &Value {
        &self.params
    }

    /// Answers with a tool result. The caller is gone when the deadline already passed;
    /// that is not an error.
    pub fn respond(self, value: Value) {
        let _ = self.reply.send(value);
    }

    /// Answers with a failure. `reply.rs` turns an object carrying `error` into a tool
    /// error, which is how the agent learns what went wrong.
    pub fn respond_error(self, message: &str) {
        self.respond(json!({ "error": message }));
    }
}

/// The receiving end of the request channel.
///
/// Like `peek_acp::AgentEvents`, a `tokio::sync::mpsc` receiver is a plain future with no
/// reactor of its own, so gpui's executor can await it directly.
#[derive(Debug)]
pub struct McpRequests(mpsc::UnboundedReceiver<McpRequest>);

impl McpRequests {
    /// The next tool call, or `None` once the server is dropped.
    pub async fn next(&mut self) -> Option<McpRequest> {
        self.0.recv().await
    }
}

#[derive(Debug)]
pub(crate) struct ChannelBridge {
    requests: mpsc::UnboundedSender<McpRequest>,
}

impl ChannelBridge {
    pub(crate) fn new() -> (Self, McpRequests) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Self { requests: sender }, McpRequests(receiver))
    }
}

#[async_trait::async_trait]
impl FrontendBridge for ChannelBridge {
    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let (reply, answer) = oneshot::channel();
        let request = McpRequest {
            method: method.to_string(),
            params,
            reply,
        };
        if self.requests.send(request).is_err() {
            return Err("the canvas is gone".to_string());
        }
        match tokio::time::timeout(REPLY_TIMEOUT, answer).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err("the canvas dropped the MCP response".to_string()),
            Err(_) => Err(format!("the canvas did not answer '{method}' in time")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_request_round_trips_through_the_channel() {
        let (bridge, mut requests) = ChannelBridge::new();
        tokio::spawn(async move {
            let request = requests.next().await.expect("a request arrives");
            assert_eq!(request.method(), "get_pages");
            assert_eq!(request.params()["pageId"], "page_1");
            request.respond(json!({ "ok": true }));
        });

        let reply = bridge
            .request("get_pages", json!({ "pageId": "page_1" }))
            .await
            .expect("the canvas answers");
        assert_eq!(reply, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn an_error_reply_reaches_the_caller_as_the_error_convention() {
        let (bridge, mut requests) = ChannelBridge::new();
        tokio::spawn(async move {
            requests
                .next()
                .await
                .expect("a request arrives")
                .respond_error("page page_x not found");
        });

        let reply = bridge.request("get_page_content", json!({})).await.unwrap();
        assert_eq!(reply["error"], "page page_x not found");
    }

    #[tokio::test]
    async fn a_dropped_receiver_fails_the_call_rather_than_hanging() {
        let (bridge, requests) = ChannelBridge::new();
        drop(requests);
        let error = bridge.request("get_pages", json!({})).await.unwrap_err();
        assert_eq!(error, "the canvas is gone");
    }

    #[tokio::test]
    async fn a_request_dropped_without_an_answer_fails_the_call() {
        let (bridge, mut requests) = ChannelBridge::new();
        tokio::spawn(async move {
            drop(requests.next().await);
        });
        let error = bridge.request("get_pages", json!({})).await.unwrap_err();
        assert_eq!(error, "the canvas dropped the MCP response");
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_canvas_times_out_with_the_method_named() {
        let (bridge, mut requests) = ChannelBridge::new();
        // Hold the request past the deadline without answering it.
        tokio::spawn(async move {
            let held = requests.next().await;
            tokio::time::sleep(Duration::from_secs(600)).await;
            drop(held);
        });
        let error = bridge.request("create_page", json!({})).await.unwrap_err();
        assert_eq!(error, "the canvas did not answer 'create_page' in time");
    }
}
