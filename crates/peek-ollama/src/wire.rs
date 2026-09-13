//! The request and response shapes `/api/chat` speaks.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool as the model is told about it: the `OpenAI` function shape Ollama borrowed.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: ToolSchema,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// A JSON Schema object describing the arguments.
    pub parameters: Value,
}

impl Tool {
    #[must_use]
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            kind: "function",
            function: ToolSchema {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// One turn in the conversation as the model sees it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct WireMessage {
    pub(crate) role: &'static str,
    pub(crate) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WireToolCall {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) function: WireToolFunction,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WireToolFunction {
    pub(crate) name: String,
    /// An object, not a string: Ollama takes the arguments already parsed.
    pub(crate) arguments: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) tools: Vec<Tool>,
    pub(crate) stream: bool,
    /// How long the server keeps the model resident. The reference's value.
    pub(crate) keep_alive: &'static str,
    /// Reasoning models otherwise interleave `<think>` blocks into the answer.
    pub(crate) think: bool,
    pub(crate) options: Options,
}

#[derive(Debug, Serialize)]
pub(crate) struct Options {
    pub(crate) num_thread: u32,
}

/// One line of the NDJSON stream.
#[derive(Debug, Deserialize)]
pub(crate) struct ChatChunk {
    #[serde(default)]
    pub(crate) message: Option<ChunkMessage>,
    #[serde(default)]
    pub(crate) done: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkMessage {
    #[serde(default)]
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<ChunkToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkToolCall {
    pub(crate) function: ChunkToolFunction,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkToolFunction {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) arguments: Value,
}
