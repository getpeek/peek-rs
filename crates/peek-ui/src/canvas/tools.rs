//! Answering one canvas tool call against the live document.
//!
//! Almost all of the work is in [`peek_canvas::tools`], which needs no window. Three things are
//! left here because they genuinely do: the two tools that read the connection rather than the
//! document, flying the camera to a target the executor computed, and reconciling the view when
//! a tool switched pages.

use gpui_kit::{Context, Window};
use peek_canvas::Camera;
use peek_canvas::camera::FitOptions;
use peek_canvas::flight::durations;
use peek_canvas::tools::{CameraMove, ToolCall, execute};
use serde_json::{Value, json};

use super::CanvasView;
use crate::database::Database;

impl CanvasView {
    /// Runs a tool call and returns the reply the bridge should send back.
    ///
    /// Takes a [`ToolCall`] rather than an `McpRequest` so a test can drive the whole surface
    /// without a server or a port, and so the agent node's own loop can share this one path.
    ///
    /// Synchronous by construction. The MCP bridge parks a one-shot channel on a five-second
    /// timeout, so nothing in here may await.
    pub(crate) fn run_tool(
        &mut self,
        call: ToolCall<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Value {
        if let Some(reply) = app_answered(call, cx) {
            return reply;
        }

        // The outgoing camera has to be persisted before the executor can switch pages, the same
        // ordering `switch_to_page` uses for a click on a tab.
        self.commit_viewport(cx);
        let page_before = self.document.read(cx).active_page_id().clone();

        let outcome = self.document.update(cx, |document, cx| {
            let outcome = execute(document, call);
            cx.notify();
            outcome
        });

        // One generic reconcile, rather than per-tool knowledge of which tools move pages: this
        // is what lets `select_nodes`, `camera_fit_node` and every create stay pure.
        if self.document.read(cx).active_page_id() != &page_before {
            self.adopt_active_page(cx);
        }
        if let Some(camera) = outcome.camera {
            self.apply_camera(camera, window, cx);
        }
        outcome.reply
    }

    fn apply_camera(&mut self, camera: CameraMove, window: &mut Window, cx: &mut Context<Self>) {
        let (target, duration) = match camera {
            CameraMove::PanTo(point) => (
                self.centred_on(point, self.camera.zoom, window),
                durations::PAN_TO,
            ),
            CameraMove::Zoom(zoom) => (
                self.camera.zoomed_about(self.pane_center(window), zoom),
                durations::SET_ZOOM,
            ),
            CameraMove::Fit(bounds) => {
                let (pane, top) = self.framing_pane(window);
                (
                    Self::below_chrome(
                        Camera::fit_bounds(bounds, pane, FitOptions::padding(0.2)),
                        top,
                    ),
                    durations::FIT_NODES,
                )
            }
        };
        self.fly_to(target, duration, window, cx);
    }
}

/// The two tools that read the connection rather than the document, and so cannot live below
/// gpui with the other nineteen.
fn app_answered(call: ToolCall<'_>, cx: &Context<CanvasView>) -> Option<Value> {
    match call.method {
        "connection_info" | "get_connection_info" => Some(Database::connection_info(cx).map_or(
            Value::Null,
            |(name, engine)| json!({ "name": name, "engine": engine.tag() }),
        )),
        "db_schema" | "get_db_schema" => {
            let tables: Option<Vec<String>> = call.params.get("tables").and_then(|value| {
                Some(
                    value
                        .as_array()?
                        .iter()
                        .filter_map(|table| table.as_str().map(str::to_string))
                        .collect(),
                )
            });
            Some(Value::String(
                Database::schema(cx).to_ddl(tables.as_deref()),
            ))
        }
        _ => None,
    }
}
