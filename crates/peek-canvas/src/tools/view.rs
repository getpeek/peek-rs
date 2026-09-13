//! Camera and selection. The three camera tools hand the view a target to fly to; the page
//! switch they and `select_nodes` perform is an ordinary document mutation.

use serde_json::{Value, json};

use super::CameraMove;
use super::params::{Params, required};
use crate::camera::Camera;
use crate::model::Document;

type Moved = Result<(Value, Option<CameraMove>), String>;

pub(super) fn camera_pan_to(params: Params<'_>) -> Moved {
    let position = required(params.point("position"), "position")?;
    Ok((json!({ "ok": true }), Some(CameraMove::PanTo(position))))
}

pub(super) fn camera_set_zoom(params: Params<'_>) -> Moved {
    let zoom = Camera::clamp_zoom(required(params.number("zoom"), "zoom")?);
    Ok((json!({ "zoom": zoom }), Some(CameraMove::Zoom(zoom))))
}

pub(super) fn camera_fit_node(document: &mut Document, params: Params<'_>) -> Moved {
    let id = required(params.node("nodeId"), "nodeId")?;
    let page = document
        .page_of(&id)
        .cloned()
        .ok_or_else(|| format!("node {id} not found"))?;

    document.switch_page(&page);
    let bounds = document
        .node(&id)
        .map(peek_document::Node::bounds)
        .ok_or_else(|| format!("node {id} not found"))?;

    Ok((
        json!({ "nodeId": id.as_str(), "pageId": page.as_str() }),
        Some(CameraMove::Fit(bounds)),
    ))
}

/// Replaces the selection, switching to the nodes' page first.
///
/// Selection is per-page, so ids that are not on the page the first id resolved to are dropped
/// rather than silently selecting nothing — the reference filters the same way.
pub(super) fn select_nodes(document: &mut Document, params: Params<'_>) -> Result<Value, String> {
    let ids = params.nodes("nodeIds");
    let Some(first) = ids.first() else {
        document.deselect_all();
        return Ok(json!({
            "selected": Vec::<&str>::new(),
            "pageId": document.active_page_id().as_str(),
        }));
    };

    let page = document
        .page_of(first)
        .cloned()
        .ok_or_else(|| format!("node {first} not found"))?;
    document.switch_page(&page);

    let selected: Vec<&str> = ids
        .iter()
        .filter(|id| document.node(id).is_some())
        .map(peek_document::NodeId::as_str)
        .collect();
    let chosen: Vec<peek_document::NodeId> = selected
        .iter()
        .map(|id| peek_document::NodeId::from(*id))
        .collect();
    document.select_only(chosen);

    Ok(json!({ "selected": selected, "pageId": page.as_str() }))
}
