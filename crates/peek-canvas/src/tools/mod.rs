//! The canvas tool surface: the 21 operations an agent can perform on a document.
//!
//! Two consumers share this. `peek-mcp` advertises the schemas and forwards a bridge method with
//! camelCase params; the agent node's local model loop calls the same methods after filling in
//! the arguments its terser schema lets the model omit ([`agent::agent_params`]). Writing the
//! executor twice is how the reference ended up with two implementations to keep in step, and
//! `~/labs/peek/src/mcp/*.ts` — which both of its backends call — is the shape being ported.
//!
//! Nineteen of the tools are pure document work. Only the *animation* of a camera move needs a
//! window, so those return a [`CameraMove`] for the view to fly; the page switches that
//! `camera_fit_node` and `select_nodes` perform are ordinary document mutations, which is what
//! keeps them here rather than in `peek-ui`.

mod agent;
mod nodes;
mod pages;
mod params;
mod regions;
mod view;

use peek_document::PageId;
use peek_document::geometry::{Point, Rect};
use serde_json::{Value, json};

use crate::model::Document;
use params::Params;

pub use agent::agent_params;

/// One call: the bridge method and its arguments.
#[derive(Debug, Clone, Copy)]
pub struct ToolCall<'a> {
    pub method: &'a str,
    pub params: &'a Value,
}

/// What the view must do once the document is updated, because flights and the pane's size are
/// the view's to know.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraMove {
    /// Centre this world point, keeping the current zoom.
    PanTo(Point),
    /// Zoom about the pane's centre. Already clamped to the canvas range.
    Zoom(f64),
    /// Frame these world bounds, never closer than 100%.
    Fit(Rect),
}

#[derive(Debug)]
pub struct ToolOutcome {
    pub reply: Value,
    pub camera: Option<CameraMove>,
}

impl ToolOutcome {
    fn new(reply: Value) -> Self {
        Self {
            reply,
            camera: None,
        }
    }

    fn with_camera(reply: Value, camera: CameraMove) -> Self {
        Self {
            reply,
            camera: Some(camera),
        }
    }
}

/// Runs one tool call against `document`.
///
/// Always answers. An unknown method and a rejected argument both come back as the `{"error"}`
/// shape rather than as nothing: the MCP bridge is waiting on a one-shot channel, and dropping a
/// reply costs the agent the full five-second timeout instead of a usable message.
pub fn execute(document: &mut Document, call: ToolCall<'_>) -> ToolOutcome {
    let params = Params(call.params);
    let outcome = match canonical(call.method) {
        "connection_info" | "db_schema" => Err(format!(
            "'{}' is answered by the app, not the document",
            call.method
        )),

        "active_page_id" => Ok(pages::active_page_id(document)),
        "pages" => Ok(pages::pages(document)),
        "page_content" => pages::page_content(document, params),
        "create_page" => pages::create_page(document, params),

        "create_query_node" => nodes::create_query_node(document, params),
        "create_vars_node" => nodes::create_vars_node(document, params),
        "create_text_node" => nodes::create_text_node(document, params),
        "update_query_node" => nodes::update_query_node(document, params),
        "update_vars_node" => nodes::update_vars_node(document, params),
        "update_text_node" => nodes::update_text_node(document, params),
        "connect_nodes" => nodes::connect_nodes(document, params),

        "group_nodes" => regions::group_nodes(document, params),
        "list_regions" => regions::list_regions(document, params),
        "add_to_region" => regions::add_to_region(document, params),
        "remove_region" => regions::remove_region(document, params),

        "camera_pan_to" => return view::camera_pan_to(params).into(),
        "camera_set_zoom" => return view::camera_set_zoom(params).into(),
        "camera_fit_node" => return view::camera_fit_node(document, params).into(),
        "select_nodes" => view::select_nodes(document, params),

        unknown => Err(format!("unknown tool '{unknown}'")),
    };
    ToolOutcome::new(reply_of(outcome))
}

/// The MCP bridge renames five read tools; a model calling the registry by tool name does not.
/// Accepting both spellings here is cheaper than a lookup table at each call site.
fn canonical(method: &str) -> &str {
    match method {
        "get_connection_info" => "connection_info",
        "get_db_schema" => "db_schema",
        "get_active_page_id" => "active_page_id",
        "get_pages" => "pages",
        "get_page_content" => "page_content",
        other => other,
    }
}

/// The single place a failure becomes the wire's error shape, so no tool hand-builds one and no
/// success payload can collide with it.
fn reply_of(outcome: Result<Value, String>) -> Value {
    match outcome {
        Ok(reply) => reply,
        Err(message) => json!({ "error": message }),
    }
}

impl From<Result<(Value, Option<CameraMove>), String>> for ToolOutcome {
    fn from(outcome: Result<(Value, Option<CameraMove>), String>) -> Self {
        match outcome {
            Ok((reply, Some(camera))) => Self::with_camera(reply, camera),
            Ok((reply, None)) => Self::new(reply),
            Err(message) => Self::new(json!({ "error": message })),
        }
    }
}

/// Commits a write tool against `page`: the edit is recorded there, the whole call is one undo
/// step, and the step is sealed before the reply goes out.
///
/// The ordering is load-bearing three times over. The transaction has to open *inside* the page
/// lens or the snapshot is taken of the wrong page; the checkpoint has to run inside it too, or
/// a call that changed nothing leaves a dead undo step on the page the user is looking at; and a
/// tool that reveals another page must switch *before* calling this, because `switch_page`
/// checkpoints and would split the step in two.
fn edit<T>(
    document: &mut Document,
    page: &PageId,
    work: impl FnOnce(&mut Document) -> T,
) -> Option<T> {
    document.on_page(page, |document| {
        let outcome = document.transaction(work);
        document.checkpoint();
        outcome
    })
}

/// The page a tool targets: the one it names, or the active one. Errors when the name is unknown
/// rather than silently retargeting the page the user is looking at.
fn target_page(document: &Document, params: Params<'_>) -> Result<PageId, String> {
    let Some(page) = params.page("pageId") else {
        return Ok(document.active_page_id().clone());
    };
    if document.inner().pages.contains_key(&page) {
        return Ok(page);
    }
    Err(format!("page {page} not found"))
}
