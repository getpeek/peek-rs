//! Page reads and `create_page`.

use serde_json::{Value, json};

use super::params::{Params, required};
use crate::model::Document;

pub(super) fn active_page_id(document: &Document) -> Value {
    json!({ "activePageId": document.active_page_id().as_str() })
}

pub(super) fn pages(document: &Document) -> Value {
    let pages: Vec<Value> = document
        .pages()
        .enumerate()
        .map(|(order, page)| json!({ "id": page.id.as_str(), "name": page.name, "order": order }))
        .collect();
    Value::Array(pages)
}

/// The page as it is persisted, minus anything carrying database rows.
///
/// A bar chart embeds its rows in `data.data` and result rows live in the sidecar, which is not
/// part of a page at all — so stripping that one field is enough to keep every row out of a
/// model's context.
pub(super) fn page_content(document: &Document, params: Params<'_>) -> Result<Value, String> {
    let id = required(params.page("pageId"), "pageId")?;
    let page = document
        .inner()
        .pages
        .get(&id)
        .ok_or_else(|| format!("page {id} not found"))?;

    let mut content = serde_json::to_value(page).map_err(|error| error.to_string())?;
    if let Some(nodes) = content.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            if let Some(data) = node.get_mut("data").and_then(Value::as_object_mut) {
                data.remove("data");
            }
        }
    }
    Ok(content)
}

pub(super) fn create_page(document: &mut Document, params: Params<'_>) -> Result<Value, String> {
    let name = required(params.text("name"), "name")?.to_string();
    let order = params
        .count("order")
        .unwrap_or(0)
        .min(document.page_count());

    // Page creation is not undoable in either app, so this bypasses `edit`.
    let id = document.add_page(Some(name.clone()), Some(order));
    Ok(json!({ "id": id.as_str(), "name": name, "order": order }))
}
