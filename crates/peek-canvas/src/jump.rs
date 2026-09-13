//! Vimium-style jump labels: badge every visible node with a short letter code, then narrow
//! the set as the code is typed. The port of `src/canvas/jump/labels.ts`.
//!
//! Only the pure half lives here — labelling, ordering and the typed-prefix state machine.
//! The scrim and badges are drawn by `peek-ui`.

use peek_document::{Node, NodeId};

use crate::{Point, Rect};

/// Home-row first, so the nearest nodes get the easiest keys to reach.
const ALPHABET: &[u8] = b"asdfghjklqwertyuiopzxcvbnm";

/// Every pair of letters, and no more: the label set is uniform-length, so past this the
/// farthest nodes simply go unlabelled.
const MAX_TARGETS: usize = ALPHABET.len() * ALPHABET.len();

/// One labelled node. `world` is its top-left corner, where the badge is anchored.
#[derive(Debug, Clone, PartialEq)]
pub struct JumpTarget {
    pub id: NodeId,
    pub label: String,
    pub world: Point,
}

/// What a letter did to the typed prefix.
#[derive(Debug, Clone, PartialEq)]
pub enum Pressed {
    /// Exactly one label matched: fly to this node and leave jump mode.
    Jump(NodeId),
    /// The prefix grew and several labels still match.
    Typed,
    /// No label starts with the prefix this letter would make, so the letter is dropped and
    /// the prefix is left as it was. Mistyping never cancels.
    Ignored,
}

/// Uniform-length labels: one character each while they fit in the alphabet, two after that.
/// Keeping the length uniform is what stops any label from being a prefix of another, which is
/// why a complete label can fire the moment it is typed.
fn labels_for_count(count: usize) -> Vec<String> {
    if count <= ALPHABET.len() {
        return ALPHABET
            .iter()
            .take(count)
            .map(|letter| char::from(*letter).to_string())
            .collect();
    }
    let mut labels = Vec::with_capacity(count.min(MAX_TARGETS));
    for first in ALPHABET {
        for second in ALPHABET {
            labels.push(format!("{}{}", char::from(*first), char::from(*second)));
            if labels.len() == count {
                return labels;
            }
        }
    }
    labels
}

#[derive(Debug, Clone)]
pub struct JumpMode {
    targets: Vec<JumpTarget>,
    typed: String,
}

impl JumpMode {
    /// Labels every node intersecting `visible`, nearest to `centre` first, so the easiest
    /// keys land on the nodes closest to where the user is already looking.
    ///
    /// `None` when nothing is visible: jump mode never opens on an empty viewport.
    #[must_use]
    pub fn new(nodes: &[Node], visible: Rect, centre: Point) -> Option<Self> {
        let mut visible_nodes: Vec<&Node> = nodes
            .iter()
            .filter(|node| node.bounds().intersects(visible))
            .collect();
        if visible_nodes.is_empty() {
            return None;
        }
        // Squared distance: the ordering is the same and the square root is not worth it.
        visible_nodes.sort_by(|a, b| {
            let distance = |node: &Node| {
                let offset = node.bounds().center() - centre;
                offset.x.mul_add(offset.x, offset.y * offset.y)
            };
            distance(a).total_cmp(&distance(b))
        });
        visible_nodes.truncate(MAX_TARGETS);

        let labels = labels_for_count(visible_nodes.len());
        let targets = visible_nodes
            .into_iter()
            .zip(labels)
            .map(|(node, label)| JumpTarget {
                id: node.id.clone(),
                label,
                world: node.position,
            })
            .collect();
        Some(Self {
            targets,
            typed: String::new(),
        })
    }

    #[must_use]
    pub fn targets(&self) -> &[JumpTarget] {
        &self.targets
    }

    #[must_use]
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Whether this target is still a candidate for what has been typed so far. Non-matching
    /// badges stay on screen, faded, rather than disappearing.
    #[must_use]
    pub fn matches(&self, target: &JumpTarget) -> bool {
        target.label.starts_with(&self.typed)
    }

    pub fn press(&mut self, letter: char) -> Pressed {
        let next = format!("{}{letter}", self.typed);
        let mut matching = self
            .targets
            .iter()
            .filter(|target| target.label.starts_with(&next));
        let Some(first) = matching.next() else {
            return Pressed::Ignored;
        };
        if matching.next().is_none() {
            return Pressed::Jump(first.id.clone());
        }
        self.typed = next;
        Pressed::Typed
    }

    /// Drops the last typed letter. Never leaves jump mode, even when the prefix is empty.
    pub fn backspace(&mut self) {
        self.typed.pop();
    }
}

#[cfg(test)]
mod tests {
    use peek_document::NodeType;

    use super::*;
    use crate::Size;

    fn node_at(x: f64, y: f64) -> Node {
        Node::new(
            NodeType::Text,
            Rect::new(Point::new(x, y), Size::new(100.0, 100.0)),
        )
    }

    fn viewport() -> Rect {
        Rect::new(Point::new(0.0, 0.0), Size::new(1000.0, 1000.0))
    }

    #[test]
    fn labels_are_uniform_length_on_both_sides_of_the_alphabet() {
        assert_eq!(labels_for_count(3), ["a", "s", "d"]);

        let many = labels_for_count(30);
        assert_eq!(many.len(), 30);
        assert!(many.iter().all(|label| label.len() == 2));
        assert_eq!(many[0], "aa");
        assert_eq!(many[26], "sa");
    }

    #[test]
    fn no_label_is_a_prefix_of_another() {
        for count in [1, 26, 27, 100] {
            let labels = labels_for_count(count);
            for label in &labels {
                let prefixes = labels
                    .iter()
                    .filter(|other| other.starts_with(label))
                    .count();
                assert_eq!(prefixes, 1, "{label} is a prefix of another label");
            }
        }
    }

    #[test]
    fn the_nearest_node_to_the_centre_gets_the_first_label() {
        let nodes = vec![node_at(800.0, 800.0), node_at(450.0, 450.0)];
        let jump = JumpMode::new(&nodes, viewport(), Point::new(500.0, 500.0)).unwrap();

        assert_eq!(jump.targets()[0].label, "a");
        assert_eq!(jump.targets()[0].id, nodes[1].id);
        assert_eq!(jump.targets()[1].label, "s");
    }

    #[test]
    fn offscreen_nodes_are_not_labelled() {
        let nodes = vec![node_at(10.0, 10.0), node_at(5000.0, 5000.0)];
        let jump = JumpMode::new(&nodes, viewport(), Point::new(500.0, 500.0)).unwrap();

        assert_eq!(jump.targets().len(), 1);
        assert_eq!(jump.targets()[0].id, nodes[0].id);
    }

    #[test]
    fn an_empty_viewport_does_not_open_jump_mode() {
        let nodes = vec![node_at(5000.0, 5000.0)];
        assert!(JumpMode::new(&nodes, viewport(), Point::new(500.0, 500.0)).is_none());
    }

    #[test]
    fn a_single_letter_label_fires_on_the_first_keystroke() {
        let nodes = vec![node_at(10.0, 10.0), node_at(400.0, 400.0)];
        let mut jump = JumpMode::new(&nodes, viewport(), Point::new(500.0, 500.0)).unwrap();
        let first = jump.targets()[0].id.clone();

        assert_eq!(jump.press('a'), Pressed::Jump(first));
    }

    #[test]
    fn a_two_letter_label_needs_both_keys() {
        let nodes: Vec<Node> = (0..30)
            .map(|index| node_at(f64::from(index) * 10.0, 0.0))
            .collect();
        let mut jump = JumpMode::new(&nodes, viewport(), Point::new(500.0, 500.0)).unwrap();
        let expected = jump.targets()[0].id.clone();

        assert_eq!(jump.press('a'), Pressed::Typed);
        assert_eq!(jump.typed(), "a");
        assert_eq!(jump.press('a'), Pressed::Jump(expected));
    }

    #[test]
    fn an_unmatched_letter_is_dropped_and_the_prefix_survives() {
        let nodes: Vec<Node> = (0..30)
            .map(|index| node_at(f64::from(index) * 10.0, 0.0))
            .collect();
        let mut jump = JumpMode::new(&nodes, viewport(), Point::new(500.0, 500.0)).unwrap();

        assert_eq!(jump.press('a'), Pressed::Typed);
        assert_eq!(jump.press('%'), Pressed::Ignored);
        assert_eq!(jump.typed(), "a");
    }

    #[test]
    fn backspace_shortens_the_prefix_and_never_underflows() {
        let nodes: Vec<Node> = (0..30)
            .map(|index| node_at(f64::from(index) * 10.0, 0.0))
            .collect();
        let mut jump = JumpMode::new(&nodes, viewport(), Point::new(500.0, 500.0)).unwrap();

        jump.press('a');
        assert_eq!(jump.typed(), "a");
        jump.backspace();
        assert_eq!(jump.typed(), "");
        jump.backspace();
        assert_eq!(jump.typed(), "");
    }

    #[test]
    fn matches_narrows_as_the_prefix_grows() {
        let nodes: Vec<Node> = (0..30)
            .map(|index| node_at(f64::from(index) * 10.0, 0.0))
            .collect();
        let mut jump = JumpMode::new(&nodes, viewport(), Point::new(500.0, 500.0)).unwrap();

        assert_eq!(
            jump.targets().iter().filter(|t| jump.matches(t)).count(),
            30
        );
        jump.press('a');
        let matching = jump.targets().iter().filter(|t| jump.matches(t)).count();
        assert!((1..30).contains(&matching), "{matching} still matching");
    }
}
