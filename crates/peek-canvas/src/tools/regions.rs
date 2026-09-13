//! The four region tools, over [`crate::regions`]'s mutators.

use peek_document::RegionStatus;
use serde_json::{Value, json};

use super::params::{Params, required};
use super::{edit, target_page};
use crate::model::Document;
use crate::regions::NewRegion;

pub(super) fn group_nodes(document: &mut Document, params: Params<'_>) -> Result<Value, String> {
    let page = target_page(document, params)?;
    let name = required(params.text("name"), "name")?.trim().to_string();
    if name.is_empty() {
        return Err("name is required".to_string());
    }
    let members = params.nodes("nodeIds");
    if members.len() < 2 {
        return Err("pass at least two node ids to group".to_string());
    }
    missing_on_page(document, &page, &members)?;

    // A region the user has not reviewed is a suggestion; `suggested: false` is how a caller
    // says it already has their blessing.
    let status = if params.flag("suggested") == Some(false) {
        RegionStatus::Confirmed
    } else {
        RegionStatus::Suggested
    };
    let desc = params.text("desc").unwrap_or_default().to_string();

    let id = edit(document, &page, |document| {
        document.group_nodes(members, NewRegion { name, desc, status })
    })
    .ok_or_else(|| format!("page {page} not found"))?;

    Ok(json!({ "regionId": id.as_str(), "pageId": page.as_str() }))
}

pub(super) fn list_regions(document: &mut Document, params: Params<'_>) -> Result<Value, String> {
    let page = target_page(document, params)?;
    document
        .on_page(&page, |document| {
            let live: Vec<&peek_document::NodeId> =
                document.nodes().iter().map(|node| &node.id).collect();

            let regions: Vec<Value> = document
                .regions()
                .iter()
                .map(|region| {
                    let members: Vec<&str> = region
                        .member_ids
                        .iter()
                        .filter(|id| live.contains(id))
                        .map(peek_document::NodeId::as_str)
                        .collect();
                    json!({
                        "regionId": region.id.as_str(),
                        "name": region.name,
                        "desc": region.desc,
                        "status": status_tag(region.status),
                        "nodeIds": members,
                    })
                })
                .collect();

            // Freehand drawings are annotation, not content, so they are never "ungrouped work".
            let ungrouped: Vec<&str> = document
                .nodes()
                .iter()
                .filter(|node| !matches!(node.kind, peek_document::NodeKind::Draw(_)))
                .filter(|node| {
                    !document
                        .regions()
                        .iter()
                        .any(|region| region.member_ids.contains(&node.id))
                })
                .map(|node| node.id.as_str())
                .collect();

            json!({
                "pageId": page.as_str(),
                "regions": regions,
                "ungroupedNodeIds": ungrouped,
            })
        })
        .ok_or_else(|| format!("page {page} not found"))
}

pub(super) fn add_to_region(document: &mut Document, params: Params<'_>) -> Result<Value, String> {
    let region = required(params.region("regionId"), "regionId")?;
    let members = params.nodes("nodeIds");
    if members.is_empty() {
        return Err("pass at least one node id to add".to_string());
    }
    let page = page_of_region(document, &region)?;
    missing_on_page(document, &page, &members)?;

    edit(document, &page, |document| {
        document.add_to_region(&region, members);
    });
    Ok(json!({ "regionId": region.as_str(), "pageId": page.as_str() }))
}

pub(super) fn remove_region(document: &mut Document, params: Params<'_>) -> Result<Value, String> {
    let region = required(params.region("regionId"), "regionId")?;
    let page = page_of_region(document, &region)?;

    edit(document, &page, |document| {
        document.remove_region(&region);
    });
    Ok(json!({ "regionId": region.as_str(), "pageId": page.as_str() }))
}

/// Regions are addressed without a page, so the id has to be hunted for across all of them.
/// Private to this file: nothing else addresses a region that way.
fn page_of_region(
    document: &Document,
    region: &peek_document::RegionId,
) -> Result<peek_document::PageId, String> {
    document
        .inner()
        .pages
        .iter()
        .find(|(_, page)| page.regions.iter().any(|candidate| &candidate.id == region))
        .map(|(id, _)| id.clone())
        .ok_or_else(|| format!("region {region} not found"))
}

fn missing_on_page(
    document: &Document,
    page: &peek_document::PageId,
    members: &[peek_document::NodeId],
) -> Result<(), String> {
    let Some(page_nodes) = document.inner().pages.get(page) else {
        return Err(format!("page {page} not found"));
    };
    let missing: Vec<&str> = members
        .iter()
        .filter(|id| !page_nodes.nodes.iter().any(|node| &&node.id == id))
        .map(peek_document::NodeId::as_str)
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!("nodes not on page {page}: {}", missing.join(", ")))
}

fn status_tag(status: RegionStatus) -> &'static str {
    match status {
        RegionStatus::Confirmed => "confirmed",
        RegionStatus::Suggested => "suggested",
    }
}
