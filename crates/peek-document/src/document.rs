use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::edge::Edge;
use crate::ids::{NodeId, PageId};
use crate::node::Node;
use crate::region::Region;

/// `version: 1` is the only document version the TypeScript app accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct DocVersion;

impl TryFrom<u32> for DocVersion {
    type Error = String;

    fn try_from(version: u32) -> Result<Self, Self::Error> {
        if version == 1 {
            Ok(Self)
        } else {
            Err(format!("unsupported document version {version}"))
        }
    }
}

impl From<DocVersion> for u32 {
    fn from(_: DocVersion) -> u32 {
        1
    }
}

/// React Flow viewport: `screen = world * zoom + (x, y)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: PageId,
    pub name: String,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub viewport: Viewport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region>,
}

impl Page {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: PageId::generate(),
            name: name.into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            viewport: Viewport::default(),
            regions: Vec::new(),
        }
    }

    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.iter().find(|node| &node.id == id)
    }

    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| &node.id == id)
    }

    /// Removes the node and every edge incident to it. Region membership is deliberately left
    /// alone: `useCanvas.ts` does the same, and regions filter against live nodes when their
    /// geometry is derived.
    pub fn remove_node(&mut self, id: &NodeId) -> bool {
        let before = self.nodes.len();
        self.nodes.retain(|node| &node.id != id);
        if self.nodes.len() == before {
            return false;
        }
        self.edges
            .retain(|edge| &edge.source != id && &edge.target != id);
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasDocument {
    pub version: DocVersion,
    pub active_page_id: PageId,
    pub page_order: Vec<PageId>,
    pub pages: BTreeMap<PageId, Page>,
}

#[derive(Debug)]
pub enum DocumentError {
    Json(serde_json::Error),
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "document json error: {error}"),
        }
    }
}

impl std::error::Error for DocumentError {}

impl CanvasDocument {
    /// One page named "Page 1" at the origin (`emptyDocument.ts`).
    #[must_use]
    pub fn empty() -> Self {
        let page = Page::new("Page 1");
        let id = page.id.clone();
        Self {
            version: DocVersion,
            active_page_id: id.clone(),
            page_order: vec![id.clone()],
            pages: BTreeMap::from([(id, page)]),
        }
    }

    /// Parses a document exactly as written by either app.
    ///
    /// # Errors
    /// Returns an error for malformed JSON or an unsupported `version`.
    pub fn from_json(json: &str) -> Result<Self, DocumentError> {
        serde_json::from_str(json).map_err(DocumentError::Json)
    }

    /// Compact JSON, like `JSON.stringify`.
    ///
    /// # Panics
    /// Never: every field is plain data with an infallible `Serialize` impl.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("document serialization is infallible")
    }

    #[must_use]
    pub fn active_page(&self) -> Option<&Page> {
        self.pages.get(&self.active_page_id)
    }

    /// Pages in display order; ids in `page_order` without a page are skipped.
    pub fn ordered_pages(&self) -> impl Iterator<Item = &Page> {
        self.page_order.iter().filter_map(|id| self.pages.get(id))
    }

    /// Appends the page and makes it active, as `useCanvas.addPage` does.
    pub fn insert_page(&mut self, page: Page) {
        let order = self.page_order.len();
        self.insert_page_at(page, order);
    }

    /// Inserts the page at `order` in the page list and makes it active. `order` is clamped to
    /// the current count, so the agent tools' `create_page` can pass an unchecked number.
    pub fn insert_page_at(&mut self, page: Page, order: usize) {
        let id = page.id.clone();
        let order = order.min(self.page_order.len());
        self.page_order.insert(order, id.clone());
        self.pages.insert(id.clone(), page);
        self.active_page_id = id;
    }

    /// Moves a page to `to` in display order, as `useCanvas.reorderPage` does: the page leaves
    /// the list before it is put back, so `to` addresses the list without it. Out-of-range
    /// indexes land in the last slot.
    pub fn reorder_page(&mut self, id: &PageId, to: usize) -> bool {
        let Some(from) = self.page_order.iter().position(|page| page == id) else {
            return false;
        };
        let to = to.min(self.page_order.len() - 1);
        if to == from {
            return false;
        }
        let id = self.page_order.remove(from);
        self.page_order.insert(to, id);
        true
    }

    pub fn rename_page(&mut self, id: &PageId, name: String) -> bool {
        let Some(page) = self.pages.get_mut(id) else {
            return false;
        };
        page.name = name;
        true
    }

    /// Refuses to remove the last page. When the active page goes, the previous page in display
    /// order becomes active.
    pub fn remove_page(&mut self, id: &PageId) -> bool {
        if self.pages.len() <= 1 || !self.pages.contains_key(id) {
            return false;
        }
        let index = self.page_order.iter().position(|page| page == id);
        self.pages.remove(id);
        if let Some(index) = index {
            self.page_order.remove(index);
        }
        if &self.active_page_id == id {
            let fallback = index.map_or(0, |index| index.saturating_sub(1));
            if let Some(next) = self
                .page_order
                .get(fallback)
                .or_else(|| self.page_order.first())
            {
                self.active_page_id = next.clone();
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{CanvasDocument, Page};

    fn names(document: &CanvasDocument) -> Vec<&str> {
        document
            .ordered_pages()
            .map(|page| page.name.as_str())
            .collect()
    }

    #[test]
    fn a_page_lands_at_the_order_it_asks_for() {
        let mut document = CanvasDocument::empty();
        document.insert_page(Page::new("second"));
        document.insert_page_at(Page::new("first"), 0);

        assert_eq!(names(&document)[0], "first");
        assert_eq!(
            document.active_page().map(|page| page.name.as_str()),
            Some("first")
        );
    }

    #[test]
    fn a_page_dropped_on_another_takes_its_slot() {
        let mut document = CanvasDocument::empty();
        document.insert_page(Page::new("b"));
        document.insert_page(Page::new("c"));
        let first = document.page_order[0].clone();

        assert!(document.reorder_page(&first, 2));
        assert_eq!(names(&document), ["b", "c", "Page 1"]);
        // Dropping a tab on itself is the gesture that changes nothing.
        let moved = document.page_order[2].clone();
        assert!(!document.reorder_page(&moved, 2));
        assert!(!document.reorder_page(&moved, 99));
    }

    /// `create_page` takes the order straight from the agent, so an out-of-range number has to
    /// append rather than panic on `Vec::insert`.
    #[test]
    fn an_order_past_the_end_appends() {
        let mut document = CanvasDocument::empty();
        document.insert_page_at(Page::new("last"), 99);

        assert_eq!(names(&document).last().copied(), Some("last"));
    }
}
