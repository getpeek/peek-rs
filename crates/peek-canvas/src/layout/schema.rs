//! The `schema` page: one [`NodeType::TableDefinition`] node per table, wired by foreign key.
//!
//! Ported from `~/labs/peek/src/command-palette/commands/viewSchema.tsx`. Nodes are seeded on a
//! circle whose radius grows with the table count, which is only a starting point — the force
//! layout in the parent module is what actually arranges them.
//!
//! Edges are ordinary document edges, as they are in the reference: `useSchemaForceLayout.ts`
//! reconciles them into `edgesAtom`, whose setter writes straight into
//! `doc.pages[activePageId].edges`. What that file derives per render is the glow class, not the
//! edges themselves. So the on-disk shape here is the reference's, not a new one.
//!
//! Ids are chosen by the caller (`schema-table-<name>`) rather than minted, so rebuilding the
//! page against a changed database replaces a table's node instead of accumulating a second one.
//!
//! A rebuild **replaces** rather than reconciles, which is what the reference does too: its
//! `deleteNode` drops every incident edge (`useCanvas.ts:85`) and `viewSchema.tsx` deletes every
//! `schema-table-*` node before recreating them, so its own reconcile finds nothing to preserve
//! at this boundary. Edge ids are derived from their endpoints, so the rebuilt edges are
//! identical; what is lost either way is selection, which is session state on both sides.

use std::collections::BTreeSet;

use peek_document::geometry::{Point, Rect, Size};
use peek_document::{NodeId, NodeType, TableDefinitionData};

use crate::model::Document;

/// The page name the schema lives on, matched literally as the reference matches it.
pub const SCHEMA_PAGE: &str = "schema";

const NODE_PREFIX: &str = "schema-table-";
const NODE_WIDTH: f64 = 450.0;
const INITIAL_SPREAD: f64 = 400.0;
/// `columns.length * 28 + 60` in `viewSchema.tsx`: a row and the header chrome.
const ROW_HEIGHT: f64 = 28.0;
const HEIGHT_PADDING: f64 = 60.0;

#[must_use]
pub fn node_id(table: &str) -> NodeId {
    NodeId::new(format!("{NODE_PREFIX}{table}"))
}

fn is_schema_node(id: &NodeId) -> bool {
    id.as_str().starts_with(NODE_PREFIX)
}

impl Document {
    /// Whether the active page is the one the schema lives on. `View::Organize` refuses to run
    /// there: the schema page is laid out by the command that builds it, and a second
    /// simulation would fight that one over every node.
    #[must_use]
    pub fn on_schema_page(&self) -> bool {
        self.active_page().name == SCHEMA_PAGE
    }

    /// Makes the `schema` page active, creating it if it is not there yet.
    pub fn open_schema_page(&mut self) {
        let existing = self
            .pages()
            .find(|page| page.name == SCHEMA_PAGE)
            .map(|page| page.id.clone());
        match existing {
            Some(id) => {
                self.switch_page(&id);
            }
            None => {
                self.add_page(Some(SCHEMA_PAGE.to_string()), None);
            }
        }
    }

    /// Replaces every table node on the active page with one per table in `tables`, connected
    /// by `references` (pairs of table names, in whichever direction the caller derived them).
    ///
    /// One undo step for the whole rebuild: dropping the old nodes and creating the new ones is
    /// a single user action, and `EditKind::Structure` would otherwise make it two.
    /// Returns the ids it created, in the order the tables came in, for the caller to frame.
    pub fn rebuild_schema_tables(
        &mut self,
        tables: &[TableDefinitionData],
        references: &[(String, String)],
    ) -> Vec<NodeId> {
        let stale: Vec<NodeId> = self
            .nodes()
            .iter()
            .map(|node| node.id.clone())
            .filter(is_schema_node)
            .collect();
        let present: BTreeSet<&str> = tables.iter().map(|table| table.table.as_str()).collect();

        self.transaction(|document| {
            document.remove_nodes(&stale);

            let created: Vec<NodeId> = tables
                .iter()
                .enumerate()
                .map(|(index, table)| {
                    let id = node_id(&table.table);
                    document.insert_node(
                        id.clone(),
                        NodeType::TableDefinition,
                        seed_bounds(index, tables.len(), table.columns.len()),
                    );
                    document.update_data::<TableDefinitionData>(&id, |data| {
                        data.clone_from(table);
                    });
                    id
                })
                .collect();

            for (source, target) in references {
                if !present.contains(source.as_str()) || !present.contains(target.as_str()) {
                    continue;
                }
                document.connect(&node_id(source), &node_id(target));
            }
            created
        })
    }
}

/// Where table `index` of `count` starts, before the force layout takes over: a circle around
/// the origin wide enough that the tables are not all inside one another.
fn seed_bounds(index: usize, count: usize, columns: usize) -> Rect {
    #[allow(
        clippy::cast_precision_loss,
        reason = "table and column counts far below 2^53 convert exactly"
    )]
    let (index, count, columns) = (index as f64, count.max(1) as f64, columns as f64);
    let radius = count.mul_add(30.0, INITIAL_SPREAD);
    let angle = index / count * std::f64::consts::TAU;
    Rect::new(
        Point::new(
            angle.cos().mul_add(radius, -NODE_WIDTH / 2.0),
            angle.sin() * radius,
        ),
        Size::new(NODE_WIDTH, columns.mul_add(ROW_HEIGHT, HEIGHT_PADDING)),
    )
}

#[cfg(test)]
mod tests {
    use super::{SCHEMA_PAGE, node_id};
    use crate::model::Document;
    use peek_document::{CanvasDocument, NodeData, NodeType, TableDefinitionData};

    fn table(name: &str, columns: &[&str]) -> TableDefinitionData {
        TableDefinitionData {
            table: name.to_string(),
            columns: columns
                .iter()
                .map(|column| ((*column).to_string(), "text".to_string()))
                .collect(),
        }
    }

    fn empty() -> Document {
        Document::load(CanvasDocument::empty())
    }

    #[test]
    fn opening_the_page_creates_it_once_and_finds_it_after() {
        let mut document = empty();
        let before = document.page_count();
        document.open_schema_page();
        assert_eq!(document.page_count(), before + 1);
        assert_eq!(document.active_page().name, SCHEMA_PAGE);

        let id = document.active_page_id().clone();
        let other = document.add_page(None, None);
        document.switch_page(&other);
        document.open_schema_page();
        assert_eq!(
            document.active_page_id(),
            &id,
            "the same page, not a second"
        );
    }

    #[test]
    fn tables_become_nodes_with_stable_ids_and_foreign_key_edges() {
        let mut document = empty();
        document.open_schema_page();
        let created = document.rebuild_schema_tables(
            &[table("users", &["id"]), table("orders", &["id", "user_id"])],
            &[("users".to_string(), "orders".to_string())],
        );

        assert_eq!(created, vec![node_id("users"), node_id("orders")]);
        assert_eq!(document.nodes().len(), 2);
        assert_eq!(
            document.node(&node_id("orders")).unwrap().node_type(),
            Some(NodeType::TableDefinition)
        );
        assert_eq!(document.edges().len(), 1, "one foreign key, one edge");
    }

    #[test]
    fn a_rebuild_replaces_the_old_tables_rather_than_stacking_on_them() {
        let mut document = empty();
        document.open_schema_page();
        document.rebuild_schema_tables(&[table("users", &["id"]), table("gone", &["id"])], &[]);
        let kept = document.create_node(
            NodeType::Text,
            peek_document::geometry::Rect::new(
                peek_document::geometry::Point::new(0.0, 0.0),
                peek_document::geometry::Size::new(100.0, 100.0),
            ),
        );

        document.rebuild_schema_tables(&[table("users", &["id", "email"])], &[]);

        assert!(document.node(&node_id("gone")).is_none(), "dropped");
        assert!(document.node(&kept).is_some(), "other nodes are left alone");
        let users = document.node(&node_id("users")).expect("rebuilt");
        assert_eq!(
            TableDefinitionData::get(&users.kind).map(|data| data.columns.len()),
            Some(2),
            "and carries the new columns"
        );
    }

    /// One transaction for the whole rebuild: a user who did not mean to press it gets back to
    /// where they were in one press, not one per table.
    #[test]
    fn a_rebuild_undoes_in_one_press() {
        let mut document = empty();
        document.open_schema_page();
        document.rebuild_schema_tables(&[table("users", &["id"]), table("orders", &["id"])], &[]);
        document.checkpoint();

        assert!(document.undo());
        assert!(document.nodes().is_empty(), "every table went in one press");
    }

    #[test]
    fn a_reference_to_a_table_that_is_not_on_the_page_is_skipped() {
        let mut document = empty();
        document.open_schema_page();
        document.rebuild_schema_tables(
            &[table("users", &["id"])],
            &[
                ("users".to_string(), "elsewhere".to_string()),
                ("users".to_string(), "users".to_string()),
            ],
        );
        assert!(
            document.edges().is_empty(),
            "no edge to a missing table, and none from a table to itself"
        );
    }
}
