//! Retained per-node view state, owned by the canvas.
//!
//! Most kinds are pure functions of their `Node` and are rebuilt every frame. The ones with
//! editors or scroll positions keep an entry here, created on first render and dropped when
//! the node leaves the document.
//!
//! Entries are pruned against the document, never against visibility: a node culled by the
//! camera or scrolled off-screen must not lose what the user was typing.

use std::collections::{HashMap, HashSet};

use gpui_kit::{App, Entity, Window};
use peek_canvas::Document;
use peek_document::{Node, NodeId, NodeKind};

use super::{agent, query, result, text, variable};

/// One kind's retained state. Kinds that are a pure function of their node have no variant;
/// a kind gains one here when it grows an editor or a scroll position.
#[derive(Debug)]
pub(crate) enum NodeState {
    Agent(agent::AgentState),
    Query(query::QueryState),
    Text(text::TextState),
    Variable(variable::VariableState),
    Result(result::ResultState),
}

impl NodeState {
    /// Last rites before the state is dropped. Most kinds own nothing outside the map; the
    /// query editor holds a document in the language server and has to give it back.
    fn on_removed(&self, cx: &mut App) {
        match self {
            Self::Agent(state) => state.on_removed(cx),
            Self::Query(state) => state.on_removed(cx),
            Self::Text(_) | Self::Variable(_) | Self::Result(_) => {}
        }
    }

    /// `None` for kinds that need no retained state. `document` is handed over so a kind's
    /// editors can keep a [`gpui_kit::WeakEntity`] and write back from their own handlers.
    fn for_node(
        node: &Node,
        document: &Entity<Document>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Self> {
        match &node.kind {
            NodeKind::Agent(data) => Some(Self::Agent(agent::AgentState::new(
                &node.id, data, document, window, cx,
            ))),
            NodeKind::Query(data) => Some(Self::Query(query::QueryState::new(
                &node.id, data, document, window, cx,
            ))),
            NodeKind::Text(data) => Some(Self::Text(text::TextState::new(
                &node.id, data, document, window, cx,
            ))),
            NodeKind::Variable(_) => Some(Self::Variable(variable::VariableState::new(
                &node.id, document, window, cx,
            ))),
            NodeKind::Result(data) => Some(Self::Result(result::ResultState::new(
                &node.id, data, document, window, cx,
            ))),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct NodeStates {
    entries: HashMap<NodeId, NodeState>,
    /// The canvas' document, which never changes for the life of the view; injected once
    /// rather than threaded through `get` on every frame.
    document: Entity<Document>,
    /// The document revision the entries were last pruned against.
    pruned_at: Option<u64>,
}

impl NodeStates {
    pub(crate) fn new(document: Entity<Document>) -> Self {
        Self {
            entries: HashMap::new(),
            document,
            pruned_at: None,
        }
    }

    pub(crate) fn document(&self) -> &Entity<Document> {
        &self.document
    }

    /// The node's state, creating it on first use.
    pub(crate) fn get(
        &mut self,
        node: &Node,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<&NodeState> {
        if !self.entries.contains_key(&node.id)
            && let Some(state) = NodeState::for_node(node, &self.document, window, cx)
        {
            self.entries.insert(node.id.clone(), state);
        }
        self.entries.get(&node.id)
    }

    /// An already-created state, without creating one.
    ///
    /// What a node reduced by the camera's level of detail asks for: it is not building an
    /// editor this frame, so it must not bring one into being either, but a node that already
    /// has one keeps it. Tests reach a node's retained state through this too.
    pub(crate) fn peek(&self, node: &NodeId) -> Option<&NodeState> {
        self.entries.get(node)
    }

    /// Whether [`NodeStates::retain_live`] has anything to do at this document revision.
    ///
    /// Asked before the caller collects the live ids, because that collection is the expensive
    /// half: every route that adds or removes a node bumps the revision, so an unchanged one
    /// means the entries are already correct.
    pub(crate) fn needs_pruning(&self, revision: u64) -> bool {
        !self.entries.is_empty() && self.pruned_at != Some(revision)
    }

    /// Drops state for nodes that are no longer in the document (deleted, undone, or on
    /// another page), letting each one release whatever it holds outside the map first.
    pub(crate) fn retain_live(&mut self, live: &HashSet<NodeId>, revision: u64, cx: &mut App) {
        self.pruned_at = Some(revision);
        let dropped: Vec<NodeId> = self
            .entries
            .keys()
            .filter(|id| !live.contains(*id))
            .cloned()
            .collect();
        for id in dropped {
            if let Some(state) = self.entries.remove(&id) {
                state.on_removed(cx);
            }
        }
    }
}
