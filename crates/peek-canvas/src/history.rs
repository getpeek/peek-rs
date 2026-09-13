//! Undo/redo for one page at a time, matching `~/labs/peek/src/canvas/ui/useUndoHistory.ts`:
//! fifty snapshots of `{nodes, edges, regions}`, coalesced over 300 ms.
//!
//! Snapshots rather than a command log: [`crate::Document`]'s mutators take closures, which
//! have no inverse, and a snapshot restores correctly no matter who wrote the change — which
//! is what multiplayer will need. Neither the viewport nor the selection is captured; the
//! TypeScript app strips both, so panning never lands in the undo stack.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use peek_document::{Edge, Node, NodeId, Page, PageId, Region};

pub(crate) const UNDO_LIMIT: usize = 50;
pub(crate) const COALESCE_WINDOW: Duration = Duration::from_millis(300);

/// What a transaction is "about". Two edits coalesce only when their kinds match, so typing
/// into one node and then dragging another are always separate undo steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditKind {
    /// Nodes or edges appeared or disappeared: never coalesced with a neighbouring edit.
    Structure,
    Move,
    Resize(NodeId),
    Data(NodeId),
}

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    regions: Vec<Region>,
}

impl Snapshot {
    fn of(page: &Page) -> Self {
        Self {
            nodes: page.nodes.clone(),
            edges: page.edges.clone(),
            regions: page.regions.clone(),
        }
    }

    fn restore(self, page: &mut Page) {
        page.nodes = self.nodes;
        page.edges = self.edges;
        page.regions = self.regions;
    }
}

#[derive(Debug)]
struct Pending {
    page: PageId,
    kind: EditKind,
    at: Instant,
    before: Snapshot,
}

#[derive(Debug, Default)]
struct PageHistory {
    past: VecDeque<Snapshot>,
    future: Vec<Snapshot>,
}

#[derive(Debug, Default)]
pub(crate) struct History {
    pages: BTreeMap<PageId, PageHistory>,
    pending: Option<Pending>,
}

impl History {
    /// Called immediately *before* a mutation is applied, with the page as it still is.
    ///
    /// An open transaction for the same page and kind that is younger than the coalescing
    /// window just slides its deadline — no snapshot, no allocation — so a sixty-frame drag
    /// or a burst of keystrokes costs one entry.
    pub(crate) fn record(&mut self, now: Instant, page: &Page, kind: EditKind) {
        if let Some(pending) = &self.pending {
            let continues = pending.page == page.id
                && pending.kind == kind
                && kind != EditKind::Structure
                && now.duration_since(pending.at) <= COALESCE_WINDOW;
            if continues {
                self.slide(now);
                return;
            }
            self.seal(page);
        }
        self.pending = Some(Pending {
            page: page.id.clone(),
            kind,
            at: now,
            before: Snapshot::of(page),
        });
    }

    fn slide(&mut self, now: Instant) {
        if let Some(pending) = &mut self.pending {
            pending.at = now;
        }
    }

    /// Closes the open transaction, if any. Called at drag end, editor blur and page switch,
    /// so two deliberate edits are two undo steps even when they land milliseconds apart.
    pub(crate) fn checkpoint(&mut self, page: &Page) {
        if self.pending.is_some() {
            self.seal(page);
        }
    }

    /// `page` must be the page as it is *now*, so a transaction that changed nothing is
    /// dropped instead of leaving a dead undo step.
    fn seal(&mut self, page: &Page) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if pending.page == page.id && pending.before == Snapshot::of(page) {
            return;
        }
        let history = self.pages.entry(pending.page).or_default();
        history.past.push_back(pending.before);
        if history.past.len() > UNDO_LIMIT {
            history.past.pop_front();
        }
        history.future.clear();
    }

    pub(crate) fn can_undo(&self, page: &PageId) -> bool {
        self.pending.as_ref().is_some_and(|open| &open.page == page)
            || self.pages.get(page).is_some_and(|it| !it.past.is_empty())
    }

    pub(crate) fn can_redo(&self, page: &PageId) -> bool {
        self.pages.get(page).is_some_and(|it| !it.future.is_empty())
    }

    /// Seals any open transaction first, so `cmd-z` part-way through typing undoes the burst
    /// instead of doing nothing.
    pub(crate) fn undo(&mut self, page: &mut Page) -> bool {
        self.checkpoint(page);
        let Some(history) = self.pages.get_mut(&page.id) else {
            return false;
        };
        let Some(previous) = history.past.pop_back() else {
            return false;
        };
        history.future.push(Snapshot::of(page));
        previous.restore(page);
        true
    }

    pub(crate) fn redo(&mut self, page: &mut Page) -> bool {
        let Some(history) = self.pages.get_mut(&page.id) else {
            return false;
        };
        let Some(next) = history.future.pop() else {
            return false;
        };
        history.past.push_back(Snapshot::of(page));
        next.restore(page);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{NodeType, TextData};

    fn page() -> Page {
        Page::new("Page 1")
    }

    fn add_node(page: &mut Page) -> NodeId {
        let node = Node::new(
            NodeType::Text,
            Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 100.0)),
        );
        let id = node.id.clone();
        page.nodes.push(node);
        id
    }

    fn set_text(page: &mut Page, id: &NodeId, text: &str) {
        let node = page.node_mut(id).unwrap();
        <TextData as peek_document::NodeData>::get_mut(&mut node.kind)
            .unwrap()
            .text = text.to_string();
    }

    fn at(base: Instant, millis: u32) -> Instant {
        base + Duration::from_millis(u64::from(millis))
    }

    #[test]
    fn edits_within_three_hundred_milliseconds_coalesce() {
        let base = Instant::now();
        let mut page = page();
        let id = add_node(&mut page);
        let mut history = History::default();

        for (step, millis) in [(1, 0), (2, 100), (3, 250)] {
            history.record(at(base, millis), &page, EditKind::Data(id.clone()));
            set_text(&mut page, &id, &format!("v{step}"));
        }
        history.checkpoint(&page);

        assert!(history.undo(&mut page));
        assert_eq!(text_of(&page, &id), "", "the whole burst is one entry");
        assert!(!history.can_undo(&page.id));
    }

    #[test]
    fn an_edit_after_the_window_starts_a_new_entry() {
        let base = Instant::now();
        let mut page = page();
        let id = add_node(&mut page);
        let mut history = History::default();

        history.record(at(base, 0), &page, EditKind::Data(id.clone()));
        set_text(&mut page, &id, "first");
        history.record(at(base, 400), &page, EditKind::Data(id.clone()));
        set_text(&mut page, &id, "second");
        history.checkpoint(&page);

        assert!(history.undo(&mut page));
        assert_eq!(text_of(&page, &id), "first");
        assert!(history.undo(&mut page));
        assert_eq!(text_of(&page, &id), "");
    }

    #[test]
    fn a_different_edit_kind_seals_the_previous_transaction() {
        let base = Instant::now();
        let mut page = page();
        let id = add_node(&mut page);
        let mut history = History::default();

        history.record(at(base, 0), &page, EditKind::Data(id.clone()));
        set_text(&mut page, &id, "typed");
        history.record(at(base, 10), &page, EditKind::Move);
        page.node_mut(&id).unwrap().position = Point::new(50.0, 50.0);
        history.checkpoint(&page);

        assert!(history.undo(&mut page));
        assert_eq!(page.node(&id).unwrap().position, Point::new(0.0, 0.0));
        assert_eq!(text_of(&page, &id), "typed", "the move undid alone");
    }

    #[test]
    fn checkpoint_makes_two_drags_two_entries() {
        let base = Instant::now();
        let mut page = page();
        let id = add_node(&mut page);
        let mut history = History::default();

        for (step, millis) in [(1.0_f64, 0), (2.0, 50)] {
            history.record(at(base, millis), &page, EditKind::Move);
            page.node_mut(&id).unwrap().position = Point::new(step * 10.0, 0.0);
            history.checkpoint(&page);
        }

        assert!(history.undo(&mut page));
        assert_eq!(page.node(&id).unwrap().position, Point::new(10.0, 0.0));
        assert!(history.undo(&mut page));
        assert_eq!(page.node(&id).unwrap().position, Point::new(0.0, 0.0));
    }

    #[test]
    fn a_no_op_edit_creates_no_entry() {
        let mut page = page();
        let id = add_node(&mut page);
        let mut history = History::default();

        history.record(Instant::now(), &page, EditKind::Data(id));
        history.checkpoint(&page);

        assert!(!history.can_undo(&page.id));
    }

    #[test]
    fn the_past_is_capped_at_fifty_entries() {
        let base = Instant::now();
        let mut page = page();
        let id = add_node(&mut page);
        let mut history = History::default();

        // Sixty spaced-out moves seal sixty entries, x = 0 through 59.
        for step in 1..=60 {
            history.record(at(base, step * 1_000), &page, EditKind::Move);
            page.node_mut(&id).unwrap().position = Point::new(f64::from(step), 0.0);
        }
        history.checkpoint(&page);

        assert_eq!(history.pages[&page.id].past.len(), UNDO_LIMIT);
        // The oldest ten fell off the front, so unwinding stops at x = 10, not at x = 0.
        while history.undo(&mut page) {}
        assert_eq!(page.node(&id).unwrap().position, Point::new(10.0, 0.0));
    }

    #[test]
    fn a_new_edit_clears_the_redo_stack() {
        let base = Instant::now();
        let mut page = page();
        let id = add_node(&mut page);
        let mut history = History::default();

        history.record(at(base, 0), &page, EditKind::Move);
        page.node_mut(&id).unwrap().position = Point::new(10.0, 0.0);
        history.checkpoint(&page);
        assert!(history.undo(&mut page));
        assert!(history.can_redo(&page.id));

        history.record(at(base, 5_000), &page, EditKind::Move);
        page.node_mut(&id).unwrap().position = Point::new(99.0, 0.0);
        history.checkpoint(&page);

        assert!(!history.can_redo(&page.id));
    }

    #[test]
    fn undo_then_redo_round_trips_nodes_edges_and_regions() {
        let base = Instant::now();
        let mut page = page();
        let mut history = History::default();

        let before = page.clone();
        history.record(at(base, 0), &page, EditKind::Structure);
        let id = add_node(&mut page);
        page.edges.push(Edge::between(id.clone(), id.clone()));
        history.checkpoint(&page);
        let after = page.clone();

        assert!(history.undo(&mut page));
        assert_eq!(page.nodes, before.nodes);
        assert_eq!(page.edges, before.edges);
        assert!(history.redo(&mut page));
        assert_eq!(page.nodes, after.nodes);
        assert_eq!(page.edges, after.edges);
    }

    #[test]
    fn history_is_per_page() {
        let base = Instant::now();
        let mut first = page();
        let mut second = page();
        let id = add_node(&mut first);
        let mut history = History::default();

        history.record(at(base, 0), &first, EditKind::Move);
        first.node_mut(&id).unwrap().position = Point::new(10.0, 0.0);
        history.checkpoint(&first);

        assert!(history.can_undo(&first.id));
        assert!(!history.can_undo(&second.id));
        assert!(!history.undo(&mut second));
    }

    fn text_of(page: &Page, id: &NodeId) -> String {
        <TextData as peek_document::NodeData>::get(&page.node(id).unwrap().kind)
            .unwrap()
            .text
            .clone()
    }
}
