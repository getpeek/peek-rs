//! The tokio runtime the MCP server runs on.
//!
//! Mirrors `peek_db::Session` and `peek_acp::AgentSession`: the runtime lives in the
//! backend crate, and the UI only awaits plain futures. Dropping the [`McpServer`] shuts
//! the listener down, which is what makes the whole thing testable.

use std::fmt;
use std::sync::Arc;

use tokio::runtime::Runtime;

use crate::channel::{ChannelBridge, McpRequests};

/// A running MCP server. Dropping it stops the listener and fails any tool call still in
/// flight, which the agent reports as a failed tool rather than a hung turn.
///
/// Dropping the runtime cancels the listener task outright. Nothing here runs on
/// `spawn_blocking`, which is the only thing a runtime drop waits for, so this never
/// blocks the main thread.
pub struct McpServer {
    #[expect(
        dead_code,
        reason = "held to keep the runtime alive for the listener task"
    )]
    runtime: Runtime,
    port: u16,
}

impl fmt::Debug for McpServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpServer")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl McpServer {
    /// Binds the server on loopback and starts serving.
    ///
    /// Returns the channel every tool call arrives on; the caller drains it on the main
    /// thread and answers each request against the document.
    ///
    /// The socket is bound here, synchronously, rather than inside the spawned task: a port
    /// already in use has to be an error the caller can report, and `port(0)` has to resolve to
    /// the port the OS actually chose before [`McpServer::url`] can name it.
    ///
    /// # Errors
    /// Returns an error if the port cannot be bound or the tokio runtime cannot be built.
    pub fn serve(port: u16) -> std::io::Result<(Self, McpRequests)> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("peek-mcp")
            .enable_all()
            .build()?;
        let (bridge, requests) = ChannelBridge::new();
        runtime.spawn(async move {
            if let Err(error) = crate::server::serve(listener, Arc::new(bridge)).await {
                log::error!("peek: the MCP server stopped: {error}");
            }
        });
        Ok((Self { runtime, port }, requests))
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The URL an agent connects to, as passed to an ACP session's `mcp_http_servers`.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port 0 lets the OS pick, so the test never collides with a real server — and the URL has
    /// to name the port it picked, not the zero that was asked for, or an agent cannot connect.
    #[test]
    fn the_url_names_the_port_that_was_actually_bound() {
        let (server, _requests) = McpServer::serve(0).expect("loopback binds");
        assert_ne!(server.port(), 0, "the OS chose a real port");
        assert_eq!(server.url(), format!("http://127.0.0.1:{}/", server.port()));
    }

    #[test]
    fn a_port_already_in_use_is_reported_rather_than_hidden() {
        let (first, _requests) = McpServer::serve(0).expect("loopback binds");
        assert!(McpServer::serve(first.port()).is_err());
    }
}
