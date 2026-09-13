//! The conversation as the caller builds it, and what comes back.

use serde_json::Value;

use crate::wire::{Tool, WireMessage, WireToolCall, WireToolFunction};

/// A tool the model asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Ollama does not reliably return ids, so one is minted per call and the caller pairs the
    /// result back by it — the reference generates a uuid in the same place.
    pub id: String,
    pub name: String,
    pub args: Value,
}

/// One piece of a streamed turn.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatDelta {
    /// A token of the answer.
    Chunk(String),
    /// The tools the model asked for, once the turn has ended.
    Calls(Vec<ToolCall>),
    /// The turn ended. Carries the answer in full, so a caller that did not accumulate the
    /// chunks does not have to.
    Done(String),
    /// The request failed. Terminal, like `Done`.
    Failed(String),
}

/// The conversation handed to the model, built by the caller.
#[derive(Debug, Clone, Default)]
pub struct Chat {
    pub(crate) messages: Vec<WireMessage>,
    pub(crate) tools: Vec<Tool>,
}

impl Chat {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = tools;
        self
    }

    #[must_use]
    pub fn system(mut self, content: impl Into<String>) -> Self {
        self.push("system", content.into(), None);
        self
    }

    #[must_use]
    pub fn user(mut self, content: impl Into<String>) -> Self {
        self.push("user", content.into(), None);
        self
    }

    #[must_use]
    pub fn assistant(mut self, content: impl Into<String>) -> Self {
        self.push("assistant", content.into(), None);
        self
    }

    /// An assistant turn that asked for tools.
    #[must_use]
    pub fn assistant_calls(mut self, content: impl Into<String>, calls: &[ToolCall]) -> Self {
        let calls = calls
            .iter()
            .map(|call| WireToolCall {
                kind: "function",
                function: WireToolFunction {
                    name: call.name.clone(),
                    arguments: call.args.clone(),
                },
            })
            .collect();
        self.push("assistant", content.into(), Some(calls));
        self
    }

    /// What a tool returned. Ollama matches results to calls positionally, so the order these
    /// are pushed in has to be the order the calls were made in.
    #[must_use]
    pub fn tool_result(mut self, content: impl Into<String>) -> Self {
        self.push("tool", content.into(), None);
        self
    }

    fn push(&mut self, role: &'static str, content: String, tool_calls: Option<Vec<WireToolCall>>) {
        self.messages.push(WireMessage {
            role,
            content,
            tool_calls,
        });
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}
