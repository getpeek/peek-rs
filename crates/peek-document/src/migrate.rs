use crate::document::CanvasDocument;
use crate::node::NodeKind;

/// Load-time clean-up mirroring `useLoadDocument.migrateAndHydrate`: a stale `isRunning`
/// left by a crash is cleared, unknown node kinds are dropped, and the active page id is
/// repaired. Returns human-readable notes for the log.
pub fn normalize(document: &mut CanvasDocument) -> Vec<String> {
    let mut notes = Vec::new();

    for page in document.pages.values_mut() {
        let before = page.nodes.len();
        page.nodes.retain(|node| node.kind != NodeKind::Unknown);
        let dropped = before - page.nodes.len();
        if dropped > 0 {
            notes.push(format!(
                "page {}: dropped {dropped} node(s) of unknown type",
                page.name
            ));
        }
        for node in &mut page.nodes {
            if let NodeKind::Query(data) = &mut node.kind {
                data.is_running = None;
            }
        }
    }

    if !document.pages.contains_key(&document.active_page_id) {
        let fallback = document
            .page_order
            .iter()
            .find(|id| document.pages.contains_key(*id))
            .or_else(|| document.pages.keys().next())
            .cloned();
        if let Some(id) = fallback {
            notes.push(format!(
                "active page {} missing, falling back to {id}",
                document.active_page_id
            ));
            document.active_page_id = id;
        }
    }

    notes
}
