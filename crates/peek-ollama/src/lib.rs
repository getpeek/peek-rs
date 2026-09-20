//! A chat client for an Ollama-compatible server.
//!
//! Peek talks to `POST {url}/api/chat` and reads its **NDJSON** stream: one JSON object per
//! line, no `data:` prefix and no terminating sentinel — the last line simply carries
//! `"done": true`. The reference reaches the same endpoint from the webview through `LangChain`;
//! doing it here instead means a remote server no longer has to serve CORS headers to the
//! window, and stopping a turn actually cancels the request rather than abandoning it.
//!
//! Mirrors `peek_db::Session` and `peek_acp::AgentSession`: the runtime lives in this crate,
//! and the UI only ever awaits plain futures.

mod chat;
mod session;
mod wire;

pub use chat::{Chat, ChatDelta, ToolCall};
pub use session::{Ask, OllamaError, OllamaSession, Turn};
pub use wire::{Tool, ToolSchema};
