//! Forking an agent conversation: `~/labs/peek/src/canvas/nodes/Agent/useForkConversation.ts`.

use peek_document::geometry::{Point, Rect};
use peek_document::{AgentData, NodeData, NodeId, NodeType};

use crate::model::Document;

/// The gap the fork leaves beside the node it came from, in world units.
const FORK_GAP: f64 = 50.0;

impl Document {
    /// Spawns a sibling agent node carrying `source`'s conversation, edged from it and selected.
    ///
    /// The messages are copied, not shared, so the two histories diverge from this point. For an
    /// ACP conversation the fork is a new node id and therefore a new session with no agent-side
    /// history — only the transcript Peek renders carries over.
    ///
    /// `None` when `source` is not an agent node. One undo step.
    pub fn fork_agent(&mut self, source: &NodeId) -> Option<NodeId> {
        let node = self.node(source)?;
        let data = AgentData::get(&node.kind)?.clone();
        let size = node.size();
        let origin = node.position + Point::new(size.width + FORK_GAP, 0.0);
        let source = source.clone();

        Some(self.transaction(|document| {
            let fork = document.create_node(NodeType::Agent, Rect::new(origin, size));
            document.update_data::<AgentData>(&fork, |target| *target = data);
            document.connect(&source, &fork);
            document.select_only([fork.clone()]);
            fork
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::Document;
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{
        AgentData, AgentMessage, AgentProvider, CanvasDocument, Node, NodeData, NodeId, NodeKind,
        NodeType, TextData,
    };

    fn document() -> Document {
        let mut persisted = CanvasDocument::empty();
        let page = persisted
            .pages
            .get_mut(&persisted.active_page_id)
            .expect("the empty document has one page");

        let mut agent = Node::new(
            NodeType::Agent,
            Rect::new(Point::new(100.0, 200.0), Size::new(540.0, 400.0)),
        );
        agent.id = NodeId::from("agent_1");
        agent.kind = NodeKind::Agent(AgentData {
            query: "seed".to_string(),
            messages: vec![AgentMessage::new("user", "hello".to_string(), 1)],
            provider: Some(AgentProvider::Acp),
        });
        page.nodes.push(agent);

        let mut text = Node::new(
            NodeType::Text,
            Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 50.0)),
        );
        text.id = NodeId::from("text_1");
        text.kind = NodeKind::Text(TextData::default());
        page.nodes.push(text);

        Document::load(persisted)
    }

    #[test]
    fn a_fork_sits_beside_its_source_at_the_same_size() {
        let mut document = document();
        let fork = document
            .fork_agent(&NodeId::from("agent_1"))
            .expect("agent_1 is an agent node");

        let node = document.node(&fork).expect("the fork was placed");
        assert_eq!(node.position, Point::new(100.0 + 540.0 + 50.0, 200.0));
        assert_eq!(node.size(), Size::new(540.0, 400.0));
    }

    #[test]
    fn a_fork_carries_the_conversation_but_does_not_share_it() {
        let mut document = document();
        let source = NodeId::from("agent_1");
        let fork = document.fork_agent(&source).expect("agent_1 is an agent");

        document.update_data::<AgentData>(&fork, |data| {
            data.messages
                .push(AgentMessage::new("user", "only mine".to_string(), 2));
        });

        let forked = AgentData::get(&document.node(&fork).unwrap().kind).unwrap();
        let original = AgentData::get(&document.node(&source).unwrap().kind).unwrap();
        assert_eq!(forked.messages.len(), 2);
        assert_eq!(original.messages.len(), 1, "the source is untouched");
        assert_eq!(forked.provider, Some(AgentProvider::Acp));
    }

    #[test]
    fn a_fork_is_edged_from_its_source_and_selected() {
        let mut document = document();
        let source = NodeId::from("agent_1");
        let fork = document.fork_agent(&source).expect("agent_1 is an agent");

        assert!(
            document
                .edges()
                .iter()
                .any(|edge| edge.source == source && edge.target == fork)
        );
        assert_eq!(document.selected().iter().collect::<Vec<_>>(), vec![&fork]);
    }

    #[test]
    fn forking_something_else_is_refused() {
        let mut document = document();
        assert!(document.fork_agent(&NodeId::from("text_1")).is_none());
        assert!(document.fork_agent(&NodeId::from("nope")).is_none());
    }

    #[test]
    fn a_fork_is_one_undo_step() {
        let mut document = document();
        let before = document.nodes().len();
        document.fork_agent(&NodeId::from("agent_1")).unwrap();
        document.checkpoint();

        assert!(document.undo());
        assert_eq!(document.nodes().len(), before);
        assert!(document.edges().is_empty(), "the edge went with it");
    }
}
