//! Peek's ACP (Agent Client Protocol) client.
//!
//! Lets Peek act as an ACP *client* that spawns an external coding agent (e.g.
//! Claude Code via `@agentclientprotocol/claude-agent-acp`) as a subprocess and
//! drives it over JSON-RPC on stdio. One [`AcpConnection`] hosts many sessions
//! (one per Agent node), so conversations stay isolated. The agent reaches
//! Peek's canvas tools through Peek's own MCP server, forwarded per session.
//!
//! Tauri-free by design (like the sibling `peek-mcp`/`peek-lsp` crates): it
//! depends only on the [`AcpHost`] trait it defines, which the app implements to
//! stream updates and permission prompts to the webview.

mod config;
mod connection;
mod events;
mod host;
mod session;
mod shell_path;
mod transcript;
mod update;

pub use config::AgentLaunch;
pub use connection::{AcpConnection, SessionInfo};
pub use events::{AgentEvent, AgentEvents, PermissionId, PermissionOption, PermissionRequest};
pub use host::AcpHost;
pub use peek_document::{AgentMessage, PlanEntry};
pub use session::{AcpError, AgentSession, Pending};
pub use transcript::{Change, Preview, Transcript};
pub use update::{AcpUpdate, ToolCallUpdate};
