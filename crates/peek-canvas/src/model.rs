//! The session document: the persisted [`CanvasDocument`] plus selection (never persisted)
//! and a revision counter (bumped on every persisted mutation, so autosave can debounce on
//! it). Mirrors the subset of the TypeScript `CanvasApi` that milestones 1–2 need.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use peek_document::geometry::{Point, Rect, Size};
use peek_document::{
    CanvasDocument, DrawData, Edge, EdgeId, Node, NodeData, NodeId, NodeType, Page, PageId,
    ResultSet, ResultSidecar, VariableData, Viewport,
};

use crate::history::{EditKind, History};
use crate::regions::GroupPlan;
use crate::scope::{HistoryScope, RegionScope, Scope};

/// `useDrawTool.ts`'s three constants. The width is document data rather than a theme role, and
/// the colour is the CSS token the frozen format stores, resolved against the live theme when
/// the node paints — so a stroke follows a theme change.
pub const DRAW_STROKE_WIDTH: f64 = 4.0;
const DRAW_PADDING: f64 = DRAW_STROKE_WIDTH * 2.0;
pub const DRAW_COLOR: &str = "var(--pk-fg)";
/// gpui's `MouseMoveEvent` carries no stylus pressure, and the reference records
/// `e.pressure || 0.5` — a constant for every mouse stroke. The renderer runs with
/// `simulate_pressure`, which derives width from sample spacing, so the stored value is never
/// read back and a constant is faithful.
const DRAW_PRESSURE: f64 = 0.5;

/// The box a stroke is committed into: the samples' own extent, inset by [`DRAW_PADDING`].
///
/// The padding is the reference's, and it is a hair tight — `perfect-freehand`'s outline
/// reaches `0.75 * size` = 12 world units from the centreline at Peek's `thinning`, so a fast
/// wide stroke clips against its own edge in both apps. Widening it here would render
/// differently from the TypeScript app for the same document, which is the thing that must not
/// happen.
fn stroke_bounds(samples: &[Point]) -> Option<Rect> {
    let (first, rest) = samples.split_first()?;
    if rest.is_empty() {
        return None;
    }
    let extent = rest
        .iter()
        .fold(Rect::new(*first, Size::new(0.0, 0.0)), |extent, sample| {
            extent.union(Rect::new(*sample, Size::new(0.0, 0.0)))
        });
    Some(Rect::new(
        Point::new(extent.min().x - DRAW_PADDING, extent.min().y - DRAW_PADDING),
        Size::new(
            extent.max().x - extent.min().x + DRAW_PADDING * 2.0,
            extent.max().y - extent.min().y + DRAW_PADDING * 2.0,
        ),
    ))
}

#[derive(Debug)]
pub struct Document {
    persisted: CanvasDocument,
    selection: BTreeSet<NodeId>,
    /// Edges select separately from nodes and never reach a history snapshot: `Snapshot`
    /// compares by value, so a selection stored on `Edge` would turn selecting into an undo
    /// step and make undo restore an old selection.
    edge_selection: BTreeSet<EdgeId>,
    revision: u64,
    history: History,
    /// Set while [`Document::transaction`] is open, so the mutations it calls join that undo
    /// step instead of each sealing it and opening another. Without it, placing a result would
    /// be four undo steps — insert, connect, data, clear-error — and undoing a run would take
    /// four presses.
    in_transaction: bool,
    /// Bumped only by [`Document::set_result`]. Separate from `revision` because the rows live
    /// in their own file: one counter would make every query re-write the document and every
    /// document edit re-write megabytes of rows.
    results_revision: u64,
    /// Nodes a finished run asked the view to frame, drained by the canvas the next time the
    /// document notifies. Session state, like the selections: a camera flight is neither an
    /// undo step nor anything the file records.
    framing: Vec<NodeId>,
    /// Result rows, keyed by result node id.
    ///
    /// Session state, like the selections above and for the same reason: `history::Snapshot`
    /// compares by value, so rows in a snapshot would make every undo copy megabytes and make
    /// re-running a query an undoable edit. They live in their own file on disk
    /// (`<connection>.results.json`) and are re-fetched by running the query again.
    results: ResultSidecar,
}

impl Document {
    /// Wraps a loaded document, seeding both selections from the persisted `selected` flags of
    /// the active page (they are dropped again on write).
    #[must_use]
    pub fn load(document: CanvasDocument) -> Self {
        let page = document.active_page();
        let selection = page
            .map(|page| {
                page.nodes
                    .iter()
                    .filter(|node| node.selected)
                    .map(|node| node.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let edge_selection = page
            .map(|page| {
                page.edges
                    .iter()
                    .filter(|edge| edge.selected)
                    .map(|edge| edge.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            persisted: document,
            selection,
            edge_selection,
            revision: 0,
            history: History::default(),
            results: ResultSidecar::default(),
            results_revision: 0,
            in_transaction: false,
            framing: Vec::new(),
        }
    }

    /// Increments on every change to the rows, so the results autosave can debounce on it.
    #[must_use]
    pub fn results_revision(&self) -> u64 {
        self.results_revision
    }

    /// Adopts the rows sidecar loaded alongside the document.
    ///
    /// Does **not** bump the revision: rows are not persisted with the document, so adopting
    /// them must not schedule a document autosave.
    pub fn adopt_results(&mut self, results: ResultSidecar) {
        self.results = results;
    }

    /// The rows for a result node, or `None` until its query has run in some session.
    #[must_use]
    pub fn result(&self, id: &NodeId) -> Option<&Arc<ResultSet>> {
        self.results.get(id)
    }

    #[must_use]
    pub fn results(&self) -> &ResultSidecar {
        &self.results
    }

    /// Stores a query's rows, replacing whatever the result node held before.
    ///
    /// Bumps the results revision so the results autosave debounces on it, exactly as a document
    /// edit does for the document.
    pub fn set_result(&mut self, id: NodeId, rows: impl Into<Arc<ResultSet>>) {
        self.results.insert(id, rows);
        self.results_revision += 1;
    }

    #[must_use]
    pub fn inner(&self) -> &CanvasDocument {
        &self.persisted
    }

    /// Increments on every change that would be persisted.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn touch(&mut self) {
        self.revision += 1;
    }

    /// Opens or extends an undo transaction. The single site that reads the clock, so
    /// [`History`]'s own tests can drive it with synthetic instants.
    pub(crate) fn begin(&mut self, kind: EditKind) {
        if self.in_transaction {
            return;
        }
        let id = self.persisted.active_page_id.clone();
        if let Some(page) = self.persisted.pages.get(&id) {
            self.history.record(Instant::now(), page, kind);
        }
    }

    /// Seals the open undo transaction: drag end, node-editor blur, page switch.
    pub fn checkpoint(&mut self) {
        let id = self.persisted.active_page_id.clone();
        if let Some(page) = self.persisted.pages.get(&id) {
            self.history.checkpoint(page);
        }
    }

    pub fn undo(&mut self) -> bool {
        self.apply_history(true)
    }

    pub fn redo(&mut self) -> bool {
        self.apply_history(false)
    }

    fn apply_history(&mut self, backwards: bool) -> bool {
        let id = self.persisted.active_page_id.clone();
        let Some(page) = self.persisted.pages.get_mut(&id) else {
            return false;
        };
        let changed = if backwards {
            self.history.undo(page)
        } else {
            self.history.redo(page)
        };
        if !changed {
            return false;
        }
        self.prune_selection();
        self.touch();
        true
    }

    /// A restored snapshot may not contain every selected node or edge any more.
    fn prune_selection(&mut self) {
        let nodes: BTreeSet<NodeId> = self.nodes().iter().map(|node| node.id.clone()).collect();
        self.selection.retain(|id| nodes.contains(id));
        let edges: BTreeSet<EdgeId> = self.edges().iter().map(|edge| edge.id.clone()).collect();
        self.edge_selection.retain(|id| edges.contains(id));
    }

    fn clear_selection(&mut self) {
        self.selection.clear();
        self.edge_selection.clear();
    }

    // ---- pages -------------------------------------------------------------------------

    #[must_use]
    pub fn active_page_id(&self) -> &PageId {
        &self.persisted.active_page_id
    }

    /// # Panics
    /// Never in practice: [`peek_document::normalize`] guarantees the active page exists.
    #[must_use]
    pub fn active_page(&self) -> &Page {
        self.persisted
            .active_page()
            .expect("active page exists after normalize")
    }

    pub(crate) fn active_page_mut(&mut self) -> &mut Page {
        let id = self.persisted.active_page_id.clone();
        self.persisted
            .pages
            .get_mut(&id)
            .expect("active page exists after normalize")
    }

    pub fn pages(&self) -> impl Iterator<Item = &Page> {
        self.persisted.ordered_pages()
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.persisted.page_order.len()
    }

    /// Switches the active page (persisted as `activePageId`) and clears the selection.
    pub fn switch_page(&mut self, id: &PageId) -> bool {
        self.checkpoint();
        if !self.persisted.pages.contains_key(id) || &self.persisted.active_page_id == id {
            return false;
        }
        self.persisted.active_page_id = id.clone();
        self.clear_selection();
        self.touch();
        true
    }

    /// The page `offset` steps from the active one in `pageOrder`, wrapping around.
    #[must_use]
    pub fn neighbour_page(&self, offset: isize) -> Option<&PageId> {
        let order = &self.persisted.page_order;
        let current = order
            .iter()
            .position(|id| id == &self.persisted.active_page_id)?;
        let len = isize::try_from(order.len()).ok()?;
        let index = (isize::try_from(current).ok()? + offset).rem_euclid(len);
        order.get(usize::try_from(index).ok()?)
    }

    // ---- nodes (read) ------------------------------------------------------------------

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.active_page().nodes
    }

    #[must_use]
    pub fn edges(&self) -> &[Edge] {
        &self.active_page().edges
    }

    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes().iter().find(|node| &node.id == id)
    }

    /// Union of the bounds of `ids` that exist on the active page.
    pub fn bounds_of<'a>(&self, ids: impl IntoIterator<Item = &'a NodeId>) -> Option<Rect> {
        ids.into_iter()
            .filter_map(|id| self.node(id))
            .map(Node::bounds)
            .reduce(Rect::union)
    }

    /// Union of every node on the active page (what "fit view" frames).
    #[must_use]
    pub fn content_bounds(&self) -> Option<Rect> {
        self.nodes().iter().map(Node::bounds).reduce(Rect::union)
    }

    // ---- framing (session only) --------------------------------------------------------

    /// Asks the view to fly the camera so `ids` are all in frame.
    ///
    /// A request rather than a call because the camera needs the pane it is framing into, which
    /// only the view knows — the same split [`crate::tools::CameraMove`] makes for the canvas
    /// tools. The latest request wins: a second run landing before the view has drawn should
    /// frame what it placed, not what the first one did.
    pub fn request_framing(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.framing = ids.into_iter().collect();
    }

    /// Takes the pending framing request, leaving none behind.
    pub fn take_framing(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.framing)
    }

    // ---- selection (session only) ------------------------------------------------------

    #[must_use]
    pub fn selected(&self) -> &BTreeSet<NodeId> {
        &self.selection
    }

    #[must_use]
    pub fn is_selected(&self, id: &NodeId) -> bool {
        self.selection.contains(id)
    }

    /// Replaces the selection. Returns whether anything changed.
    /// Selecting nodes clears the edge selection, the way React Flow's `addSelectedNodes`
    /// does. The edge set is part of the no-op test, or a marquee sweeping the same nodes
    /// would report "unchanged" while leaving an edge ringed.
    pub fn select_only(&mut self, ids: impl IntoIterator<Item = NodeId>) -> bool {
        let next: BTreeSet<NodeId> = ids.into_iter().collect();
        if next == self.selection && self.edge_selection.is_empty() {
            return false;
        }
        self.selection = next;
        self.edge_selection.clear();
        true
    }

    pub fn extend_selection(&mut self, ids: impl IntoIterator<Item = NodeId>) -> bool {
        let before = self.selection.len();
        self.selection.extend(ids);
        self.selection.len() != before
    }

    pub fn toggle_selected(&mut self, id: &NodeId) -> bool {
        if !self.selection.remove(id) {
            self.selection.insert(id.clone());
        }
        true
    }

    pub fn deselect_all(&mut self) -> bool {
        if self.selection.is_empty() && self.edge_selection.is_empty() {
            return false;
        }
        self.clear_selection();
        true
    }

    #[must_use]
    pub fn selected_edges(&self) -> &BTreeSet<EdgeId> {
        &self.edge_selection
    }

    #[must_use]
    pub fn is_edge_selected(&self, id: &EdgeId) -> bool {
        self.edge_selection.contains(id)
    }

    /// The mirror of [`Document::select_only`]: picking an edge clears the node selection.
    pub fn select_edge_only(&mut self, id: EdgeId) -> bool {
        let next = BTreeSet::from([id]);
        if self.selection.is_empty() && self.edge_selection == next {
            return false;
        }
        self.clear_selection();
        self.edge_selection = next;
        true
    }

    /// Shift-clicking an edge toggles it within the edge set and leaves nodes alone.
    pub fn toggle_edge_selected(&mut self, id: &EdgeId) -> bool {
        if !self.edge_selection.remove(id) {
            self.edge_selection.insert(id.clone());
        }
        true
    }

    pub fn select_all(&mut self) -> bool {
        let all: Vec<NodeId> = self.nodes().iter().map(|node| node.id.clone()).collect();
        self.select_only(all)
    }

    // ---- geometry mutations ------------------------------------------------------------

    pub fn set_position(&mut self, id: &NodeId, position: Point) {
        self.begin(EditKind::Move);
        if let Some(node) = self.node_mut(id) {
            node.position = position;
            self.touch();
        }
    }

    pub fn translate_nodes(&mut self, ids: &[NodeId], delta: Point) {
        self.begin(EditKind::Move);
        let mut moved = false;
        for node in &mut self.active_page_mut().nodes {
            if ids.contains(&node.id) {
                node.position = node.position + delta;
                moved = true;
            }
        }
        if moved {
            self.touch();
        }
    }

    pub fn set_size(&mut self, id: &NodeId, size: Size) {
        // Opening the transaction first would leave one open over a node that isn't there —
        // a deferred write landing after a delete does exactly that.
        if self.node(id).is_none() {
            return;
        }
        self.begin(EditKind::Resize(id.clone()));
        self.write_size(id, size);
    }

    /// A size the node derived from its own content rather than one the user dragged: the Text
    /// node widening to fit the line being typed into it.
    ///
    /// Persisted like any other geometry, so autosave picks it up, but it opens **no** undo
    /// transaction of its own. Two reasons. It is not a user edit, so it should not be a step
    /// the user has to undo past; and recording it would cut a real edit in half, because
    /// `EditKind::Resize` differs from the `EditKind::Data` of the typing that caused it and
    /// would seal that transaction mid-burst. Folding it into whatever transaction is open
    /// instead means undoing the typing also restores the width — which is what
    /// `useUndoHistory.ts` does, having no edit-kind concept at all.
    pub fn set_intrinsic_size(&mut self, id: &NodeId, size: Size) {
        if self.node(id).is_none() {
            return;
        }
        self.write_size(id, size);
    }

    fn write_size(&mut self, id: &NodeId, size: Size) {
        if let Some(node) = self.node_mut(id) {
            let min = node.node_type().map_or(size, NodeType::min_size);
            node.width = Some(size.width.max(min.width));
            node.height = Some(size.height.max(min.height));
            node.measured = None;
            self.touch();
        }
    }

    /// Position and size together, for resize handles anchored on a top or left edge, where
    /// the origin moves with the size.
    pub fn set_bounds(&mut self, id: &NodeId, bounds: Rect) {
        self.begin(EditKind::Resize(id.clone()));
        self.write_bounds(id, bounds);
    }

    /// The live bounds of the node a placement drag is sizing.
    ///
    /// It opens no transaction of its own, so the `Structure` one [`Document::create_node`]
    /// opened on the drag's first frame stays open until the release checkpoints it: one undo
    /// removes the node, rather than one per size it passed through.
    pub fn resize_placement(&mut self, id: &NodeId, bounds: Rect) {
        self.write_bounds(id, bounds);
    }

    /// Drops the node a cancelled placement drag created, along with the undo transaction its
    /// creation opened: escaping mid-drag leaves the page as it was, with nothing to undo.
    pub fn cancel_placement(&mut self, id: &NodeId) {
        if self.active_page_mut().remove_node(id) {
            self.prune_selection();
            self.touch();
        }
        self.history.discard();
    }

    fn write_bounds(&mut self, id: &NodeId, bounds: Rect) {
        if let Some(node) = self.node_mut(id) {
            let min = node.node_type().map_or(bounds.size, NodeType::min_size);
            node.position = bounds.origin;
            node.width = Some(bounds.size.width.max(min.width));
            node.height = Some(bounds.size.height.max(min.height));
            node.measured = None;
            self.touch();
        }
    }

    /// Runs `work` as a single undo step, whatever mutations it calls inside.
    ///
    /// The seam a canvas tool call commits through: one call is one user-visible action, so
    /// create + set data + connect have to undo in one press. `EditKind::Structure` never
    /// coalesces, so two tool calls are always two steps and neither can fold into the user's
    /// own typing burst.
    pub fn transaction<T>(&mut self, work: impl FnOnce(&mut Self) -> T) -> T {
        self.transaction_of(EditKind::Structure, work)
    }

    /// Runs `work` as a single undo step, whatever mutations it calls inside.
    ///
    /// Re-entrant: a nested call joins the transaction already open rather than starting one.
    pub(crate) fn transaction_of<T>(
        &mut self,
        kind: EditKind,
        work: impl FnOnce(&mut Self) -> T,
    ) -> T {
        if self.in_transaction {
            return work(self);
        }
        self.begin(kind);
        self.in_transaction = true;
        let outcome = work(self);
        self.in_transaction = false;
        outcome
    }

    /// Adds a node under an id the caller chose, rather than a freshly minted one.
    ///
    /// Result and error nodes are addressed by construction from the query that produced them
    /// (`<query>-result-0`, `<query>-error`), which is what makes re-running a query update the
    /// same node instead of littering the canvas with new ones.
    pub(crate) fn insert_node(&mut self, id: NodeId, node_type: NodeType, bounds: Rect) {
        self.begin(EditKind::Structure);
        let mut node = Node::new(node_type, bounds);
        node.id = id;
        self.active_page_mut().nodes.push(node);
        self.touch();
    }

    // ---- content mutations -------------------------------------------------------------

    /// Creates a node on the active page. The document mints the id and the empty per-kind
    /// data; the caller owns the geometry, which is clamped to the kind's minimum.
    ///
    /// A new query node is connected from every global variable node on the page, as
    /// `useCanvas.addNode` does.
    pub fn create_node(&mut self, node_type: NodeType, bounds: Rect) -> NodeId {
        self.begin(EditKind::Structure);
        let node = Node::new(node_type, bounds);
        let id = node.id.clone();
        self.active_page_mut().nodes.push(node);

        if node_type == NodeType::Query {
            for source in self.global_variable_nodes() {
                self.push_edge(source, id.clone());
            }
        }
        self.touch();
        id
    }

    /// Commits one freehand stroke as a draw node, the way `useDrawTool.ts` does: the bounding
    /// box of the samples, inset by [`DRAW_PADDING`] on every side, with the points stored
    /// relative to the node's own origin.
    ///
    /// `None` below two samples, so a click draws nothing however it arrives — the gesture
    /// reducer, the MCP bridge and the tests all reach this one guard.
    ///
    /// The colour is set explicitly because it is a trap: `NodeKind::empty` yields `makeNode`'s
    /// palette default of `"white"`, and `useDrawTool` overrides it to `var(--pk-fg)`. Without
    /// this every stroke would be white in every theme.
    pub fn create_drawing(&mut self, samples: &[Point]) -> Option<NodeId> {
        let bounds = stroke_bounds(samples)?;
        let origin = bounds.origin;
        let points = samples
            .iter()
            .map(|sample| [sample.x - origin.x, sample.y - origin.y, DRAW_PRESSURE])
            .collect();
        Some(self.transaction_of(EditKind::Structure, |document| {
            let id = document.create_node(NodeType::Draw, bounds);
            document.update_data::<DrawData>(&id, |data| {
                data.points = points;
                data.stroke_width = DRAW_STROKE_WIDTH;
                data.color = DRAW_COLOR.to_string();
            });
            id
        }))
    }

    /// Adds copies of `nodes` to the active page under fresh ids, offset by `delta`, and
    /// selects them — the paste half of `usePeekHotkeys`'s cut/copy/paste.
    ///
    /// One transaction, so a paste of six nodes is one undo press. The copies keep their
    /// per-kind payload, which is the whole point: a pasted query still holds its SQL.
    pub fn paste_nodes(&mut self, nodes: &[Node], delta: Point) -> Vec<NodeId> {
        self.transaction(|document| {
            let mut pasted = Vec::with_capacity(nodes.len());
            for node in nodes {
                // A kind this build does not know has no id prefix to mint from, and pasting
                // it under the id it came with would collide with the node it was copied from.
                let Some(node_type) = node.node_type() else {
                    continue;
                };
                let mut copy = node.clone();
                copy.id = NodeId::for_type(node_type);
                copy.position = Point::new(node.position.x + delta.x, node.position.y + delta.y);
                copy.selected = false;
                pasted.push(copy.id.clone());
                document.active_page_mut().nodes.push(copy);
            }
            document.touch();
            document.select_only(pasted.clone());
            pasted
        })
    }

    /// Removes the nodes and their incident edges. Returns how many existed.
    pub fn remove_nodes(&mut self, ids: &[NodeId]) -> usize {
        self.remove(ids, &[])
    }

    pub fn delete_selection(&mut self) -> usize {
        let nodes: Vec<NodeId> = self.selection.iter().cloned().collect();
        let edges: Vec<EdgeId> = self.edge_selection.iter().cloned().collect();
        self.remove(&nodes, &edges)
    }

    /// The one structural removal. Nodes and edges go in a single transaction because
    /// [`History`] never coalesces `EditKind::Structure`, so two calls would be two undo
    /// steps for one Delete. Edges already dropped with their node are not counted twice:
    /// the tally comes from the length delta, not from the requested ids.
    fn remove(&mut self, nodes: &[NodeId], edges: &[EdgeId]) -> usize {
        self.begin(EditKind::Structure);
        let page = self.active_page_mut();
        let mut removed = nodes.iter().filter(|id| page.remove_node(id)).count();
        let before = page.edges.len();
        page.edges.retain(|edge| !edges.contains(&edge.id));
        removed += before - page.edges.len();
        if removed > 0 {
            // Inside the transaction above, so a region losing its last node goes with it in
            // the same undo step rather than leaving a label over nothing.
            self.prune_empty_regions();
            self.prune_selection();
            self.touch();
        }
        removed
    }

    /// Edits one kind's payload in place. `D` names the kind, so a node of another kind is a
    /// no-op returning `false` and nothing is recorded.
    pub fn update_data<D: NodeData>(&mut self, id: &NodeId, edit: impl FnOnce(&mut D)) -> bool {
        if self.node(id).and_then(|node| D::get(&node.kind)).is_none() {
            return false;
        }
        self.begin(EditKind::Data(id.clone()));
        let Some(node) = self.node_mut(id) else {
            return false;
        };
        let Some(data) = D::get_mut(&mut node.kind) else {
            return false;
        };
        edit(data);
        self.touch();
        true
    }

    // ---- edges -------------------------------------------------------------------------

    /// Idempotent, and refuses an edge from a node to itself.
    pub fn connect(&mut self, source: &NodeId, target: &NodeId) -> bool {
        if source == target {
            return false;
        }
        let id = EdgeId::between(source, target);
        if self.edges().iter().any(|edge| edge.id == id) {
            return false;
        }
        self.begin(EditKind::Structure);
        self.push_edge(source.clone(), target.clone());
        self.touch();
        true
    }

    pub fn disconnect(&mut self, id: &EdgeId) -> bool {
        if !self.edges().iter().any(|edge| &edge.id == id) {
            return false;
        }
        self.begin(EditKind::Structure);
        self.active_page_mut().edges.retain(|edge| &edge.id != id);
        self.touch();
        true
    }

    fn push_edge(&mut self, source: NodeId, target: NodeId) {
        self.active_page_mut()
            .edges
            .push(Edge::between(source, target));
    }

    fn global_variable_nodes(&self) -> Vec<NodeId> {
        self.nodes()
            .iter()
            .filter(|node| {
                VariableData::get(&node.kind).is_some_and(|data| data.is_global == Some(true))
            })
            .map(|node| node.id.clone())
            .collect()
    }

    // ---- page mutations ----------------------------------------------------------------

    /// Adds a page named "Page {n}" unless a name is given, and makes it active. `order` is its
    /// zero-based slot in the page list, clamped to the current count; `None` appends.
    pub fn add_page(&mut self, name: Option<String>, order: Option<usize>) -> PageId {
        self.checkpoint();
        let name = name.unwrap_or_else(|| format!("Page {}", self.page_count() + 1));
        let page = Page::new(name);
        let id = page.id.clone();
        match order {
            Some(order) => self.persisted.insert_page_at(page, order),
            None => self.persisted.insert_page(page),
        }
        self.clear_selection();
        self.touch();
        id
    }

    /// The page a node lives on, wherever it is. The active-page accessors cannot answer this,
    /// and a tool may name a node the user is not looking at.
    #[must_use]
    pub fn page_of(&self, node: &NodeId) -> Option<&PageId> {
        self.persisted
            .pages
            .iter()
            .find(|(_, page)| page.nodes.iter().any(|candidate| &candidate.id == node))
            .map(|(id, _)| id)
    }

    /// Runs `work` with `page` temporarily active, then puts the active page back — how a tool
    /// edits a node on a page the user is not looking at without yanking the view to it.
    ///
    /// A lens, not a page switch: no selection clear, no revision bump, no checkpoint. An edit
    /// inside is recorded against `page`, so a transaction must be opened *inside* this, never
    /// around it. `None` when the page does not exist.
    ///
    /// Deleting inside the scope would prune the selection against the wrong page; nothing does
    /// today, and a caller that wants to should switch pages properly instead.
    pub fn on_page<T>(&mut self, page: &PageId, work: impl FnOnce(&mut Self) -> T) -> Option<T> {
        if !self.persisted.pages.contains_key(page) {
            return None;
        }
        let restore = std::mem::replace(&mut self.persisted.active_page_id, page.clone());
        let outcome = work(self);
        self.persisted.active_page_id = restore;
        Some(outcome)
    }

    pub fn rename_page(&mut self, id: &PageId, name: String) -> bool {
        if !self.persisted.rename_page(id, name) {
            return false;
        }
        self.touch();
        true
    }

    /// Refuses to delete the last page. Page deletion is not undoable, matching the
    /// TypeScript app — which is why it asks for confirmation first.
    pub fn delete_page(&mut self, id: &PageId) -> bool {
        if !self.persisted.remove_page(id) {
            return false;
        }
        self.clear_selection();
        self.touch();
        true
    }

    fn node_mut(&mut self, id: &NodeId) -> Option<&mut Node> {
        self.active_page_mut()
            .nodes
            .iter_mut()
            .find(|node| &node.id == id)
    }

    // ---- viewport ----------------------------------------------------------------------

    #[must_use]
    pub fn viewport(&self) -> Viewport {
        self.active_page().viewport
    }

    /// Persists the camera. Call only at gesture end / flight end, never per frame.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        if self.active_page().viewport == viewport {
            return;
        }
        self.active_page_mut().viewport = viewport;
        self.touch();
    }

    // ---- scope -------------------------------------------------------------------------

    /// Regions with at least one member still on the page — what the picker would list. A
    /// region whose members are all gone has no box and so is not on the canvas at all.
    fn live_region_count(&self) -> usize {
        self.regions()
            .iter()
            .filter(|region| {
                region
                    .member_ids
                    .iter()
                    .any(|id| self.nodes().iter().any(|node| &node.id == id))
            })
            .count()
    }

    #[must_use]
    pub fn scope(&self) -> Scope {
        let kind_count = |kind: NodeType| {
            self.selection
                .iter()
                .filter_map(|id| self.node(id))
                .filter(|node| node.node_type() == Some(kind))
                .count()
        };
        Scope {
            selected: self.selection.len(),
            selected_edges: self.edge_selection.len(),
            selected_queries: kind_count(NodeType::Query),
            selected_results: kind_count(NodeType::Result),
            selected_agents: kind_count(NodeType::Agent),
            queries: self
                .nodes()
                .iter()
                .filter(|node| node.node_type() == Some(NodeType::Query))
                .count(),
            pages: self.page_count(),
            history: HistoryScope {
                can_undo: self.history.can_undo(self.active_page_id()),
                can_redo: self.history.can_redo(self.active_page_id()),
            },
            regions: {
                let plan = self.group_plan();
                RegionScope {
                    count: self.live_region_count(),
                    can_group: plan != GroupPlan::Unavailable,
                    can_fold: matches!(plan, GroupPlan::FoldInto(_)),
                    can_ungroup: !self.grouped_selection().is_empty(),
                    ungrouped: self.ungrouped_count(),
                    groupable: self.groupable_count(),
                }
            },
            ..Scope::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use peek_document::{NodeKind, QueryData, TextData, VariableRow, VariableValue};

    fn node(id: &str, x: f64, kind: NodeKind) -> Node {
        Node {
            id: NodeId::from(id),
            position: Point::new(x, 0.0),
            width: Some(100.0),
            height: Some(50.0),
            measured: None,
            selected: false,
            kind,
        }
    }

    fn document() -> Document {
        let mut doc = CanvasDocument::empty();
        let second = Page::new("Page 2");
        let second_id = second.id.clone();
        doc.page_order.push(second_id.clone());
        doc.pages.insert(second_id, second);
        let page = doc.pages.get_mut(&doc.active_page_id).unwrap();
        page.nodes
            .push(node("q1", 0.0, NodeKind::Query(QueryData::default())));
        page.nodes
            .push(node("t1", 500.0, NodeKind::Text(TextData::default())));
        Document::load(doc)
    }

    /// `document()` plus one edge from the query node to the text node.
    fn wired() -> (Document, EdgeId) {
        let mut document = document();
        assert!(document.connect(&NodeId::from("q1"), &NodeId::from("t1")));
        document.checkpoint();
        let id = document.edges()[0].id.clone();
        (document, id)
    }

    #[test]
    fn a_drawing_is_committed_into_its_padded_bounding_box() {
        let mut document = Document::load(CanvasDocument::empty());
        let samples = [
            Point::new(100.0, 60.0),
            Point::new(130.0, 40.0),
            Point::new(160.0, 90.0),
        ];

        let id = document.create_drawing(&samples).expect("three samples");
        let node = document.node(&id).expect("the node was added");

        assert!(id.as_str().starts_with("draw_"));
        assert_eq!(
            node.bounds(),
            Rect::new(Point::new(92.0, 32.0), Size::new(76.0, 66.0)),
            "the samples' extent inset by 8 on every side"
        );
        let NodeKind::Draw(data) = &node.kind else {
            panic!("expected a drawing, got {:?}", node.kind);
        };
        assert_eq!(
            data.points,
            vec![[8.0, 28.0, 0.5], [38.0, 8.0, 0.5], [68.0, 58.0, 0.5]],
            "points are relative to the node's origin, so the extent starts at the padding"
        );
        assert!((data.stroke_width - DRAW_STROKE_WIDTH).abs() < f64::EPSILON);
        assert_eq!(
            data.color, "var(--pk-fg)",
            "not `NodeKind::empty`'s white, which would be invisible in a light theme"
        );
    }

    #[test]
    fn a_drawing_needs_two_samples_and_undoes_in_one_step() {
        let mut document = Document::load(CanvasDocument::empty());

        assert!(document.create_drawing(&[]).is_none());
        assert!(
            document.create_drawing(&[Point::new(10.0, 10.0)]).is_none(),
            "a click draws nothing"
        );
        assert!(document.nodes().is_empty(), "and adds no node either");

        document
            .create_drawing(&[Point::new(0.0, 0.0), Point::new(20.0, 20.0)])
            .expect("two samples");
        document.checkpoint();
        assert_eq!(document.nodes().len(), 1);

        // The node and its data are one transaction: without it the `update_data` would seal a
        // second entry and undoing a stroke would leave an empty drawing behind.
        assert!(document.undo());
        assert!(document.nodes().is_empty(), "one press, one stroke gone");
    }

    #[test]
    fn selecting_an_edge_clears_the_nodes_and_a_node_click_clears_the_edge() {
        let (mut document, edge) = wired();
        assert!(document.select_only([NodeId::from("q1")]));

        assert!(document.select_edge_only(edge.clone()));
        assert!(document.selected().is_empty(), "the node selection went");
        assert!(document.is_edge_selected(&edge));

        assert!(document.select_only([NodeId::from("q1")]));
        assert!(
            document.selected_edges().is_empty(),
            "and picking a node takes the edge selection with it"
        );
    }

    #[test]
    fn select_only_clears_the_edge_even_when_the_nodes_are_unchanged() {
        // The marquee calls this every frame; reporting "no change" here would leave an edge
        // ringed with no repaint to clear it.
        let (mut document, edge) = wired();
        document.select_only([NodeId::from("q1")]);
        document.toggle_edge_selected(&edge);

        assert!(document.select_only([NodeId::from("q1")]));
        assert!(document.selected_edges().is_empty());
    }

    #[test]
    fn shift_clicking_keeps_the_other_kind_selected() {
        let (mut document, edge) = wired();
        document.select_only([NodeId::from("q1")]);

        document.toggle_edge_selected(&edge);
        assert_eq!(document.selected().len(), 1, "the node survives");
        assert!(document.is_edge_selected(&edge));

        document.toggle_selected(&NodeId::from("t1"));
        assert!(document.is_edge_selected(&edge), "and the edge survives");

        document.toggle_edge_selected(&edge);
        assert!(
            !document.is_edge_selected(&edge),
            "toggling again clears it"
        );
    }

    #[test]
    fn edge_selection_does_not_bump_the_revision() {
        let (mut document, edge) = wired();
        let revision = document.revision();

        document.select_edge_only(edge);
        document.deselect_all();

        assert_eq!(
            document.revision(),
            revision,
            "autosave keys off the revision, so selection must never touch it"
        );
    }

    #[test]
    fn deselect_all_clears_both_and_reports_no_change_when_already_empty() {
        let (mut document, edge) = wired();
        document.select_only([NodeId::from("q1")]);
        document.toggle_edge_selected(&edge);

        assert!(document.deselect_all());
        assert!(document.selected().is_empty() && document.selected_edges().is_empty());
        assert!(!document.deselect_all(), "no-op reports no change");
    }

    /// A region whose members are all gone has no box, so the picker would not list it — and
    /// `RegionScope::count` is what the picker's command reads.
    #[test]
    fn scope_counts_only_regions_that_still_have_a_member() {
        let mut document = document();
        let alive = document.nodes()[0].id.clone();
        document.group_nodes(
            vec![alive],
            crate::NewRegion {
                name: "alive".to_string(),
                desc: String::new(),
                status: peek_document::RegionStatus::Confirmed,
            },
        );
        document.group_nodes(
            vec![NodeId::from("ghost")],
            crate::NewRegion {
                name: "dead".to_string(),
                desc: String::new(),
                status: peek_document::RegionStatus::Confirmed,
            },
        );

        assert_eq!(document.regions().len(), 2);
        assert_eq!(document.scope().regions.count, 1);
    }

    #[test]
    fn scope_counts_selected_edges() {
        let (mut document, edge) = wired();
        document.select_edge_only(edge);

        let scope = document.scope();
        assert_eq!(scope.selected_edges, 1);
        assert_eq!(scope.selected, 0, "framing has nothing to frame");
    }

    #[test]
    fn deleting_a_node_and_an_unrelated_edge_is_one_undo_step() {
        let mut document = document();
        // Wired up behind history's back, so the delete below is the first undoable edit.
        let page = document.active_page_mut();
        page.nodes
            .push(node("t2", 900.0, NodeKind::Text(TextData::default())));
        page.edges
            .push(Edge::between(NodeId::from("t1"), NodeId::from("t2")));
        let edge = document.edges()[0].id.clone();

        document.select_only([NodeId::from("q1")]);
        document.toggle_edge_selected(&edge);
        assert_eq!(document.delete_selection(), 2, "one node and one edge");
        document.checkpoint();

        assert!(document.undo(), "a single step restores both");
        assert_eq!(document.nodes().len(), 3);
        assert_eq!(document.edges().len(), 1);
        assert!(!document.undo(), "and there is no second step to take");
    }

    #[test]
    fn an_edge_deleted_with_its_node_is_not_counted_twice() {
        let (mut document, edge) = wired();
        document.select_only([NodeId::from("q1")]);
        document.toggle_edge_selected(&edge);

        assert_eq!(
            document.delete_selection(),
            1,
            "the edge went with its node, so only the node is tallied"
        );
        assert!(document.edges().is_empty());
        assert!(document.selected_edges().is_empty(), "and was pruned");
    }

    #[test]
    fn undo_prunes_the_edge_selection_to_surviving_edges() {
        let (mut document, edge) = wired();
        document.select_edge_only(edge.clone());

        // Undo rolls back to before the edge existed.
        assert!(document.undo());

        assert!(
            document.selected_edges().is_empty(),
            "an edge that no longer exists cannot stay selected"
        );
        assert!(!document.is_edge_selected(&edge));
    }

    #[test]
    fn switching_pages_clears_the_edge_selection() {
        let (mut document, edge) = wired();
        document.select_edge_only(edge);
        let other = document.inner().page_order[1].clone();

        assert!(document.switch_page(&other));
        assert!(document.selected_edges().is_empty());
    }

    #[test]
    fn selection_does_not_bump_revision_but_geometry_does() {
        let mut document = document();
        assert!(document.select_all());
        assert!(!document.select_all(), "no-op reports no change");
        assert_eq!(document.revision(), 0);
        assert_eq!(document.scope().selected_queries, 1);

        document.translate_nodes(&[NodeId::from("q1")], Point::new(10.0, 5.0));
        assert_eq!(document.revision(), 1);
        assert_eq!(
            document.node(&NodeId::from("q1")).unwrap().position,
            Point::new(10.0, 5.0)
        );

        document.set_size(&NodeId::from("q1"), Size::new(1.0, 1.0));
        assert_eq!(
            document.node(&NodeId::from("q1")).unwrap().size(),
            NodeType::Query.min_size()
        );
    }

    #[test]
    fn content_bounds_span_all_nodes() {
        let document = document();
        let bounds = document.content_bounds().unwrap();
        assert_eq!(bounds.origin, Point::new(0.0, 0.0));
        assert_eq!(bounds.size, Size::new(600.0, 50.0));
    }

    #[test]
    fn switching_pages_clears_selection_and_wraps() {
        let mut document = document();
        document.select_all();
        let next = document.neighbour_page(1).unwrap().clone();
        assert!(document.switch_page(&next));
        assert!(document.selected().is_empty());
        assert_eq!(document.revision(), 1);
        let back = document.neighbour_page(1).unwrap().clone();
        assert_ne!(back, next, "two pages wrap around");
        assert!(!document.switch_page(&next), "already active");
    }

    #[test]
    fn viewport_is_only_written_when_changed() {
        let mut document = document();
        document.set_viewport(Viewport::default());
        assert_eq!(document.revision(), 0);
        document.set_viewport(Viewport {
            x: 1.0,
            y: 2.0,
            zoom: 0.5,
        });
        assert_eq!(document.revision(), 1);
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect::new(Point::new(x, y), Size::new(width, height))
    }

    #[test]
    fn create_node_mints_a_prefixed_id_and_clamps_to_min_size() {
        let mut document = document();
        let id = document.create_node(NodeType::Text, rect(10.0, 20.0, 5.0, 5.0));

        let node = document.node(&id).unwrap();
        assert!(id.as_str().starts_with("text_"));
        assert_eq!(node.position, Point::new(10.0, 20.0));
        assert_eq!(node.size(), NodeType::Text.min_size());
        assert_eq!(document.revision(), 1);
    }

    /// A pasted node is a new node that kept its payload. Minting the id is what stops a paste
    /// from colliding with the node it was copied from; keeping the payload is what makes it a
    /// paste rather than "create an empty one of the same kind".
    #[test]
    fn pasting_mints_ids_keeps_the_payload_and_selects_the_copies() {
        let mut document = document();
        let mut source = node("q1", 0.0, NodeKind::Query(QueryData::default()));
        if let NodeKind::Query(data) = &mut source.kind {
            data.query = "select 1".to_string();
        }

        let pasted = document.paste_nodes(&[source], Point::new(40.0, 60.0));

        let copy = document.node(&pasted[0]).expect("the copy is on the page");
        assert!(copy.id.as_str().starts_with("query_"), "{}", copy.id);
        assert_eq!(copy.position, Point::new(40.0, 60.0));
        let NodeKind::Query(data) = &copy.kind else {
            panic!("a pasted query is still a query");
        };
        assert_eq!(data.query, "select 1");
        assert_eq!(
            document.selected().iter().cloned().collect::<Vec<_>>(),
            pasted,
            "and the paste leaves its copies selected, as the reference does"
        );
    }

    #[test]
    fn pasting_several_nodes_undoes_in_one_press() {
        let mut document = document();
        let copied = document.nodes().to_vec();
        assert_eq!(copied.len(), 2);

        document.paste_nodes(&copied, Point::new(10.0, 10.0));
        document.checkpoint();
        assert_eq!(document.nodes().len(), 4);

        assert!(document.undo());
        assert_eq!(document.nodes().len(), 2);
        assert!(!document.undo(), "and the paste was the only step");
    }

    #[test]
    fn creating_a_query_connects_global_variable_nodes() {
        let mut document = document();
        let global = |is_global| {
            NodeKind::Variable(VariableData {
                rows: vec![VariableRow {
                    name: "env".to_string(),
                    value: VariableValue::One("prod".to_string()),
                }],
                is_global,
            })
        };
        document
            .active_page_mut()
            .nodes
            .push(node("v_global", 0.0, global(Some(true))));
        document
            .active_page_mut()
            .nodes
            .push(node("v_local", 0.0, global(None)));

        let query = document.create_node(NodeType::Query, rect(0.0, 0.0, 350.0, 240.0));

        let sources: Vec<&str> = document
            .edges()
            .iter()
            .map(|edge| edge.source.as_str())
            .collect();
        assert_eq!(sources, vec!["v_global"]);
        assert_eq!(document.edges()[0].target, query);
    }

    #[test]
    fn removing_nodes_drops_incident_edges_and_deselects() {
        let mut document = document();
        assert!(document.connect(&NodeId::from("q1"), &NodeId::from("t1")));
        document.select_all();

        assert_eq!(document.remove_nodes(&[NodeId::from("t1")]), 1);

        assert!(
            document.edges().is_empty(),
            "incident edge went with the node"
        );
        assert_eq!(document.selected().len(), 1);
        assert_eq!(document.remove_nodes(&[NodeId::from("gone")]), 0);
    }

    #[test]
    fn delete_selection_removes_the_selection_and_empties_it() {
        let mut document = document();
        document.select_all();
        assert_eq!(document.delete_selection(), 2);
        assert!(document.nodes().is_empty());
        assert!(document.selected().is_empty());
    }

    #[test]
    fn update_data_edits_only_the_matching_kind() {
        let mut document = document();
        let before = document.revision();

        assert!(
            !document.update_data::<TextData>(&NodeId::from("q1"), |data| {
                data.text = "wrong kind".to_string();
            })
        );
        assert_eq!(document.revision(), before, "a mismatch changes nothing");

        assert!(
            document.update_data::<TextData>(&NodeId::from("t1"), |data| {
                data.text = "hello".to_string();
            })
        );
        let kind = &document.node(&NodeId::from("t1")).unwrap().kind;
        assert_eq!(TextData::get(kind).unwrap().text, "hello");
    }

    #[test]
    fn connect_is_idempotent_and_rejects_self_edges() {
        let mut document = document();
        assert!(document.connect(&NodeId::from("q1"), &NodeId::from("t1")));
        assert!(!document.connect(&NodeId::from("q1"), &NodeId::from("t1")));
        assert!(!document.connect(&NodeId::from("q1"), &NodeId::from("q1")));
        assert_eq!(document.edges().len(), 1);

        let id = document.edges()[0].id.clone();
        assert!(document.disconnect(&id));
        assert!(!document.disconnect(&id));
    }

    #[test]
    fn set_bounds_clamps_to_min_size_and_clears_measured() {
        let mut document = document();
        document.set_bounds(&NodeId::from("q1"), rect(5.0, 6.0, 10.0, 10.0));

        let node = document.node(&NodeId::from("q1")).unwrap();
        assert_eq!(node.position, Point::new(5.0, 6.0));
        assert_eq!(node.size(), NodeType::Query.min_size());
        assert!(node.measured.is_none());
    }

    #[test]
    fn add_page_appends_activates_and_names_page_n() {
        let mut document = document();
        assert_eq!(document.page_count(), 2);

        let id = document.add_page(None, None);

        assert_eq!(document.active_page_id(), &id);
        assert_eq!(document.active_page().name, "Page 3");
        assert!(document.selected().is_empty());
    }

    #[test]
    fn delete_page_refuses_the_last_page_and_falls_back_to_the_previous() {
        let mut document = document();
        let first = document.active_page_id().clone();
        let second = document.neighbour_page(1).unwrap().clone();

        assert!(document.delete_page(&first));
        assert_eq!(document.active_page_id(), &second);
        assert!(!document.delete_page(&second), "the last page stays");
    }

    #[test]
    fn undo_restores_content_but_not_the_viewport() {
        let mut document = document();
        document.set_viewport(Viewport {
            x: 40.0,
            y: 40.0,
            zoom: 2.0,
        });
        document.checkpoint();

        let id = document.create_node(NodeType::Text, rect(0.0, 0.0, 100.0, 100.0));
        document.checkpoint();
        assert_eq!(document.nodes().len(), 3);

        assert!(document.undo());
        assert_eq!(document.nodes().len(), 2);
        assert!(document.node(&id).is_none());
        assert_eq!(
            document.viewport(),
            Viewport {
                x: 40.0,
                y: 40.0,
                zoom: 2.0
            },
            "panning is not undoable"
        );

        assert!(document.redo());
        assert_eq!(document.nodes().len(), 3);
    }

    #[test]
    fn undo_prunes_the_selection_to_surviving_nodes() {
        let mut document = document();
        let id = document.create_node(NodeType::Text, rect(0.0, 0.0, 100.0, 100.0));
        document.checkpoint();
        document.select_all();
        assert_eq!(document.selected().len(), 3);

        assert!(document.undo());
        assert_eq!(document.selected().len(), 2);
        assert!(!document.is_selected(&id));
    }

    #[test]
    fn resizing_a_node_that_is_gone_opens_no_transaction() {
        let mut document = document();
        document.remove_nodes(&[NodeId::from("t1")]);
        document.checkpoint();
        let before = document.revision();

        document.set_size(&NodeId::from("t1"), Size::new(400.0, 400.0));
        document.checkpoint();

        assert_eq!(document.revision(), before, "nothing was written");
        // The stale write must not leave a transaction that swallows the next real edit.
        assert!(document.undo(), "the delete is still the top of the stack");
        assert!(document.node(&NodeId::from("t1")).is_some());
    }

    #[test]
    fn an_intrinsic_resize_is_undone_with_the_edit_that_caused_it() {
        let mut document = document();
        let id = NodeId::from("t1");
        let original = document.node(&id).unwrap().size();

        // What the Text node does: type, then widen to fit what was typed.
        document.update_data::<TextData>(&id, |data| data.text = "a much longer line".to_string());
        document.set_intrinsic_size(&id, Size::new(624.0, original.height));
        document.checkpoint();

        assert!(document.undo());
        let node = document.node(&id).unwrap();
        assert_eq!(
            TextData::get(&node.kind).unwrap().text,
            "",
            "one undo takes the whole edit"
        );
        assert_eq!(
            node.size(),
            original,
            "and the width it caused goes with it"
        );
        assert!(!document.undo(), "the grow was not a step of its own");
    }

    #[test]
    fn an_intrinsic_resize_does_not_split_a_typing_burst() {
        let mut document = document();
        let id = NodeId::from("t1");

        // A grow lands mid-burst, as it does from about the fifth character onwards.
        document.update_data::<TextData>(&id, |data| data.text = "abcde".to_string());
        document.set_intrinsic_size(&id, Size::new(500.0, 200.0));
        document.update_data::<TextData>(&id, |data| data.text = "abcdefghij".to_string());
        document.checkpoint();

        assert!(document.undo());
        assert_eq!(
            TextData::get(&document.node(&id).unwrap().kind)
                .unwrap()
                .text,
            "",
            "the burst is still one undo step"
        );
        assert!(!document.undo());
    }

    #[test]
    fn an_intrinsic_resize_still_reaches_autosave() {
        let mut document = document();
        let before = document.revision();
        document.set_intrinsic_size(&NodeId::from("t1"), Size::new(500.0, 200.0));
        assert!(document.revision() > before, "the document is dirty");
    }

    /// Rows are session state. If they bumped the document revision, every query would rewrite
    /// the document file; if they reached a history snapshot, every undo would copy megabytes
    /// and re-running a query would become an undoable edit.
    #[test]
    fn storing_rows_leaves_the_document_and_its_history_alone() {
        use peek_document::{Cell, Column, ResultSet};

        let mut document = document();
        let before = document.revision();
        let could_undo = document.scope().history.can_undo;

        let rows = ResultSet::new(vec![Column::new("id", "INT4")], vec![vec![Cell::Int(1)]]);
        document.set_result(NodeId::from("q1-result-0"), rows.clone());

        assert_eq!(document.revision(), before, "rows are not a document edit");
        assert_eq!(
            document.scope().history.can_undo,
            could_undo,
            "rows must not open an undo transaction"
        );
        assert!(
            document.results_revision() > 0,
            "the results autosave still has to see the change"
        );
        assert_eq!(
            document
                .result(&NodeId::from("q1-result-0"))
                .map(AsRef::as_ref),
            Some(&rows)
        );
    }

    #[test]
    fn a_result_node_without_rows_reads_as_none() {
        assert_eq!(document().result(&NodeId::from("never-run-result-0")), None);
    }

    #[test]
    fn a_node_is_found_on_whichever_page_holds_it() {
        let mut document = document();
        let second = document.neighbour_page(1).cloned().unwrap();
        let active = document.active_page_id().clone();

        assert_eq!(document.page_of(&NodeId::from("q1")), Some(&active));
        assert_eq!(document.page_of(&NodeId::from("nope")), None);

        document.on_page(&second, |document| {
            document.create_node(
                NodeType::Text,
                Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 50.0)),
            )
        });
        let placed = document
            .on_page(&second, |document| document.nodes()[0].id.clone())
            .unwrap();
        assert_eq!(document.page_of(&placed), Some(&second));
    }

    /// The whole point of the lens: a tool may edit a node the user is not looking at, and the
    /// view must not jump to it.
    #[test]
    fn the_lens_puts_the_active_page_back() {
        let mut document = document();
        let second = document.neighbour_page(1).cloned().unwrap();
        let before = document.active_page_id().clone();

        let seen = document
            .on_page(&second, |document| document.active_page_id().clone())
            .unwrap();

        assert_eq!(seen, second, "work runs with the page active");
        assert_eq!(document.active_page_id(), &before, "and it is put back");
    }

    #[test]
    fn the_lens_refuses_a_page_that_is_not_there() {
        let mut document = document();
        assert!(
            document
                .on_page(&PageId::from("page_nope"), |_| ())
                .is_none()
        );
    }

    /// A transaction opened inside the lens is recorded against the page it targeted, and undo
    /// stacks are per-page — so the edit is undoable *there*, and the page the user is looking
    /// at keeps its own history. A tool editing a background node must not eat the user's undo.
    #[test]
    fn an_edit_through_the_lens_undoes_on_its_own_page() {
        let mut document = document();
        let second = document.neighbour_page(1).cloned().unwrap();

        document.on_page(&second, |document| {
            document.transaction(|document| {
                document.create_node(
                    NodeType::Text,
                    Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 50.0)),
                );
            });
            document.checkpoint();
        });

        assert!(!document.undo(), "the active page has nothing to undo");

        assert!(document.switch_page(&second));
        assert_eq!(document.nodes().len(), 1);
        assert!(document.undo());
        assert!(document.nodes().is_empty());
    }

    #[test]
    fn a_page_can_be_added_at_a_chosen_order() {
        let mut document = document();
        let first = document.add_page(Some("wedged".to_string()), Some(0));

        assert_eq!(document.pages().next().map(|page| &page.id), Some(&first));
        assert_eq!(document.active_page_id(), &first);
    }
}
