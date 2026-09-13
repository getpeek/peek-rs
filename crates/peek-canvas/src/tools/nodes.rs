//! Creating, editing and connecting nodes.
//!
//! Creates reveal the page they land on; updates deliberately do not, so a tool tidying a node
//! the user is not looking at never yanks the view away from them.

use peek_document::geometry::{Point, Rect, Size};
use peek_document::{NodeData, NodeId, NodeType, PageId, QueryData, TextData, VariableData};
use serde_json::{Value, json};

use super::params::{Params, required};
use super::{edit, target_page};
use crate::model::Document;

/// `~/labs/peek/src/mcp/textNodes.ts`'s `TEXT_NODE_INITIAL_WIDTH`: a text node's width is
/// derived from its text when it renders, so creation only picks a starting point.
const TEXT_WIDTH: f64 = 100.0;

pub(super) fn create_query_node(
    document: &mut Document,
    params: Params<'_>,
) -> Result<Value, String> {
    let page = target_page(document, params)?;
    let query = required(params.text("query"), "query")?.to_string();
    let description = params.text("description").map(str::to_string);
    let bounds = placement(params, NodeType::Query);

    reveal(document, &page);
    let id = edit(document, &page, |document| {
        // `create_node` wires the page's global variable nodes to a new query itself.
        let id = document.create_node(NodeType::Query, bounds);
        document.update_data::<QueryData>(&id, |data| {
            data.query = query;
            data.description = description;
        });
        id
    })
    .ok_or_else(|| format!("page {page} not found"))?;

    Ok(placed(&id, &page))
}

pub(super) fn create_vars_node(
    document: &mut Document,
    params: Params<'_>,
) -> Result<Value, String> {
    let page = target_page(document, params)?;
    let rows = required(params.variables("variables"), "variables")??;
    let global = params.flag("global").unwrap_or(false);
    let bounds = placement(params, NodeType::Variable);

    reveal(document, &page);
    let id = edit(document, &page, |document| {
        let id = document.create_node(NodeType::Variable, bounds);
        document.update_data::<VariableData>(&id, |data| {
            data.rows = rows;
            data.is_global = global.then_some(true);
        });
        if global {
            connect_to_queries(document, &id);
        }
        id
    })
    .ok_or_else(|| format!("page {page} not found"))?;

    Ok(placed(&id, &page))
}

pub(super) fn create_text_node(
    document: &mut Document,
    params: Params<'_>,
) -> Result<Value, String> {
    let page = target_page(document, params)?;
    let text = required(params.text("text"), "text")?.to_string();
    // The height is the font size; the width is a starting point the renderer grows.
    let height = params
        .number("height")
        .unwrap_or_else(|| NodeType::Text.default_size().height);
    let origin = params.point("position").unwrap_or(Point::new(0.0, 0.0));
    let bounds = Rect::new(origin, Size::new(TEXT_WIDTH, height));

    reveal(document, &page);
    let id = edit(document, &page, |document| {
        let id = document.create_node(NodeType::Text, bounds);
        document.update_data::<TextData>(&id, |data| data.text = text);
        id
    })
    .ok_or_else(|| format!("page {page} not found"))?;

    Ok(placed(&id, &page))
}

pub(super) fn update_query_node(
    document: &mut Document,
    params: Params<'_>,
) -> Result<Value, String> {
    let (id, page) = locate::<QueryData>(document, params, NodeType::Query)?;
    let query = params.text("query").map(str::to_string);
    let description = params.text("description").map(str::to_string);
    let geometry = (params.point("position"), params.size("size"));

    edit(document, &page, |document| {
        document.update_data::<QueryData>(&id, |data| {
            if let Some(query) = query {
                data.query = query;
            }
            if description.is_some() {
                data.description = description;
            }
        });
        reshape(document, &id, geometry);
    });
    Ok(placed(&id, &page))
}

pub(super) fn update_vars_node(
    document: &mut Document,
    params: Params<'_>,
) -> Result<Value, String> {
    let (id, page) = locate::<VariableData>(document, params, NodeType::Variable)?;
    let rows = params.variables("variables").transpose()?;
    let global = params.flag("global");
    let geometry = (params.point("position"), params.size("size"));

    edit(document, &page, |document| {
        document.update_data::<VariableData>(&id, |data| {
            if let Some(rows) = rows {
                data.rows = rows;
            }
            if let Some(global) = global {
                data.is_global = global.then_some(true);
            }
        });
        // Turning `global` off deliberately leaves the edges it drew: the in-app toggle never
        // tears them down either, and an agent should not delete wiring the user may rely on.
        if global == Some(true) {
            connect_to_queries(document, &id);
        }
        reshape(document, &id, geometry);
    });
    Ok(placed(&id, &page))
}

pub(super) fn update_text_node(
    document: &mut Document,
    params: Params<'_>,
) -> Result<Value, String> {
    let (id, page) = locate::<TextData>(document, params, NodeType::Text)?;
    let text = params.text("text").map(str::to_string);
    let height = params.number("height");
    let position = params.point("position");

    edit(document, &page, |document| {
        document.update_data::<TextData>(&id, |data| {
            if let Some(text) = text {
                data.text = text;
            }
        });
        if let Some(position) = position {
            document.set_position(&id, position);
        }
        // Width is never settable on a text node; it follows the text.
        if let Some(height) = height {
            let width = document
                .node(&id)
                .map_or(TEXT_WIDTH, |node| node.size().width);
            document.set_size(&id, Size::new(width, height));
        }
    });
    Ok(placed(&id, &page))
}

/// Idempotent: an edge that already exists is a success, not a conflict, so an agent retrying a
/// step does not have to distinguish the two.
pub(super) fn connect_nodes(document: &mut Document, params: Params<'_>) -> Result<Value, String> {
    let from = required(params.node("from"), "from")?;
    let to = required(params.node("to"), "to")?;
    if from == to {
        return Err("cannot connect a node to itself".to_string());
    }

    let from_page = page_of(document, &from)?;
    let to_page = page_of(document, &to)?;
    if from_page != to_page {
        return Err(format!(
            "nodes are on different pages ({from_page}, {to_page})"
        ));
    }

    let edge = peek_document::EdgeId::between(&from, &to);
    edit(document, &from_page, |document| {
        document.connect(&from, &to);
    });
    Ok(json!({ "edgeId": edge.as_str(), "pageId": from_page.as_str() }))
}

/// Resolves `nodeId` and checks its kind before anything is written, so a tool that names the
/// wrong kind of node fails cleanly rather than half-applying.
fn locate<D: NodeData>(
    document: &Document,
    params: Params<'_>,
    node_type: NodeType,
) -> Result<(NodeId, PageId), String> {
    let id = required(params.node("nodeId"), "nodeId")?;
    let page = page_of(document, &id)?;
    let node = document
        .inner()
        .pages
        .get(&page)
        .and_then(|page| page.nodes.iter().find(|node| node.id == id))
        .ok_or_else(|| format!("node {id} not found"))?;

    if D::get(&node.kind).is_none() {
        return Err(format!("node {id} is not a {} node", node_type.as_str()));
    }
    Ok((id, page))
}

fn page_of(document: &Document, id: &NodeId) -> Result<PageId, String> {
    document
        .page_of(id)
        .cloned()
        .ok_or_else(|| format!("node {id} not found"))
}

/// Creates reveal the page they land on, matching `placeNode` in the reference.
fn reveal(document: &mut Document, page: &PageId) {
    document.switch_page(page);
}

fn placement(params: Params<'_>, node_type: NodeType) -> Rect {
    let origin = params.point("position").unwrap_or(Point::new(0.0, 0.0));
    let size = params
        .size("size")
        .unwrap_or_else(|| node_type.default_size());
    Rect::new(origin, size)
}

fn reshape(document: &mut Document, id: &NodeId, geometry: (Option<Point>, Option<Size>)) {
    if let Some(position) = geometry.0 {
        document.set_position(id, position);
    }
    if let Some(size) = geometry.1 {
        document.set_size(id, size);
    }
}

/// Wires a global variable node to every query node on its page, skipping edges already there.
fn connect_to_queries(document: &mut Document, source: &NodeId) {
    let queries: Vec<NodeId> = document
        .nodes()
        .iter()
        .filter(|node| QueryData::get(&node.kind).is_some())
        .map(|node| node.id.clone())
        .collect();
    for query in &queries {
        document.connect(source, query);
    }
}

fn placed(id: &NodeId, page: &PageId) -> Value {
    json!({ "nodeId": id.as_str(), "pageId": page.as_str() })
}
