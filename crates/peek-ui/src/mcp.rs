//! The MCP server and the loop that answers its tool calls.
//!
//! `peek-mcp` advertises the tool schemas and forwards each call as a bridge method; every one is
//! answered here, on the main thread, against the live document.

use gpui_kit::{App, Entity, Task, Window};
use peek_canvas::tools::ToolCall;
use peek_mcp::{McpRequests, McpServer};

use crate::canvas::CanvasView;

/// Owns the MCP server for as long as the workspace lives. Dropping it stops the listener, and
/// any tool call still in flight fails rather than hanging the agent.
pub(crate) struct McpBridge {
    server: McpServer,
    /// Dropping the drain ends it; it is held rather than detached so it dies with the window.
    _drain: Task<()>,
}

impl std::fmt::Debug for McpBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpBridge")
            .field("url", &self.server.url())
            .finish_non_exhaustive()
    }
}

impl McpBridge {
    /// Starts the server and the drain, or `None` when the port is taken or the runtime will not
    /// build. A failure is logged and not fatal: the canvas works without an agent driving it.
    pub(crate) fn start(
        canvas: &Entity<CanvasView>,
        port: u16,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Self> {
        let (server, requests) = match McpServer::serve(port) {
            Ok(started) => started,
            Err(error) => {
                log::error!("peek: could not start the MCP server on port {port}: {error}");
                return None;
            }
        };
        log::info!("peek: MCP server listening on {}", server.url());
        Some(Self {
            _drain: drain(canvas.clone(), requests, window, cx),
            server,
        })
    }

    /// The address an ACP session forwards to its agent.
    pub(crate) fn url(&self) -> String {
        self.server.url()
    }
}

/// Answers one request at a time, in arrival order.
///
/// Sequential on purpose. An agent's `create_query_node` followed by `connect_nodes` must not
/// interleave, and each call's undo step has to be sealed before the next arrives.
///
/// The five-second reply timeout is met by construction rather than by watching a clock: the
/// handler is one synchronous `update_in`, and every branch of the executor answers.
fn drain(
    canvas: Entity<CanvasView>,
    mut requests: McpRequests,
    window: &Window,
    cx: &App,
) -> Task<()> {
    window.spawn(cx, async move |cx| {
        while let Some(request) = requests.next().await {
            let method = request.method().to_string();
            let params = request.params().clone();
            let call = ToolCall {
                method: &method,
                params: &params,
            };
            let Ok(reply) =
                canvas.update_in(cx, |view, window, cx| view.run_tool(call, window, cx))
            else {
                // The window is gone. Dropping `request` fails that one call, which is what the
                // agent should see, rather than leaving it parked until the timeout.
                return;
            };
            request.respond(reply);
        }
    })
}
