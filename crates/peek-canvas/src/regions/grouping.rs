//! Letting a local model organize the page into regions: the port of `useAiGrouping.ts`,
//! `useGroupWithAi.ts`, `useRegroupAllWithAi.ts` and `clusterUngrouped.ts`.
//!
//! Everything here is pure. A [`Prompt`] is built from the document, the caller sends its
//! `system` and `user` text to whatever model it has, and [`Prompt::parse`] turns the reply
//! back into a [`GroupingPlan`] the document applies as one undo step. A model that cannot be
//! reached, or answers with something unusable, is not an error: [`Prompt::fallback`] clusters
//! the same nodes geometrically, which is what the reference does too.
//!
//! Both prompts are the reference's, **verbatim** — the two apps read the same documents, so
//! the same page should come back grouped the same way.

use std::collections::HashMap;

use peek_document::{Node, NodeId, NodeKind, RegionId, RegionStatus};
use serde_json::Value;

use crate::describe::describe;
use crate::model::Document;
use crate::regions::NewRegion;

/// Nodes past this many never reach the prompt — `MAX_PROMPT_NODES`.
const MAX_PROMPT_NODES: usize = 40;
const MAX_SNIPPET_CHARS: usize = 100;
const MAX_NAME_CHARS: usize = 40;
const MAX_DESC_CHARS: usize = 60;

/// Two nodes join the same geometric cluster when an edge connects them or their centres sit
/// within this many canvas pixels of each other — `PROXIMITY_PX`.
const PROXIMITY: f64 = 420.0;
/// A region of one is not a grouping; the model is held to the same rule.
const MIN_GROUP: usize = 2;

const SYSTEM_PARTITION: &str = r#"/no_think You organize a database-exploration canvas into regions (named groups of nodes).

You are given a numbered list of nodes with their kind, content and [x,y] position, plus the edges between them. Decide how to partition the nodes into regions.

How to group:
- Edges show flow (a query → its result → a chart is one thread), but being connected does NOT force nodes into one region. A single connected graph usually fans out from a root into SEVERAL distinct investigations — split each branch into its own region.
- Prefer several precise regions over one broad catch-all. If nodes cover clearly different topics (e.g. billing vs. churn vs. onboarding), they belong in different regions even when linked.
- Use spatial position as a hint: nodes clustered together in [x,y] usually belong together; a large gap suggests a boundary.
- Every node goes in exactly one region. Skip a node only if it truly fits nowhere.
- Aim for 2-6 nodes per region; name each by what it investigates.

Reply with ONLY a JSON array, no prose or markdown:
[{"name":"Short Name","desc":"one line, <=8 words","nodes":[1,2,3]}]
"name" is 2-4 words. Use the node NUMBERS from the list."#;

const SYSTEM_EXTEND: &str = r#"/no_think You maintain the regions (named groups of nodes) on a database-exploration canvas.

Some nodes are already organized into existing regions. You are given those regions, then the currently ungrouped nodes with their kind, content and [x,y] position, plus the edges between the ungrouped nodes. Decide where each ungrouped node belongs.

How to decide:
- If a node clearly fits the topic of an existing region, ADD it there — reference the region by its [R#] label.
- Otherwise group it with other ungrouped nodes into a NEW region.
- Prefer several precise regions over one broad catch-all. Edges show flow but do NOT force nodes together.
- Use spatial position as a hint. Leave a node out only if it truly fits nowhere.

Reply with ONLY a JSON array, no prose or markdown:
[{"into":"R1","nodes":[1,2]},{"name":"Short Name","desc":"one line, <=8 words","nodes":[3,4]}]
Use "into" with an existing [R#] label to extend that region (a single node is fine), or "name"+"desc" for a new region (needs at least 2 nodes). "name" is 2-4 words. Use the node NUMBERS from the ungrouped list."#;

/// Where one group of nodes ends up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignment {
    /// Folded into a region that already exists, which keeps its name.
    Existing {
        region: RegionId,
        members: Vec<NodeId>,
    },
    /// A region of its own, for the user to confirm or rename.
    New {
        name: String,
        desc: String,
        members: Vec<NodeId>,
    },
}

/// What to do to the page's regions: the assignments, and whether they are an addition or a
/// replacement.
///
/// Built only by [`Prompt`], so "replace everything" cannot be asked for by accident — it is
/// what the prompt was for or it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupingPlan {
    pub(crate) replaces: bool,
    pub(crate) assignments: Vec<Assignment>,
}

impl GroupingPlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    #[must_use]
    pub fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }
}

/// A question about one page, and everything needed to read the answer.
#[derive(Debug, Clone)]
pub struct Prompt {
    system: &'static str,
    user: String,
    /// The node each prompt number stands for; the model answers in numbers.
    numbered: Vec<NodeId>,
    /// `R1` and friends. Empty for a full regroup, which shows the model no regions.
    labelled: HashMap<String, RegionId>,
    replaces: bool,
}

impl Prompt {
    /// Organize the ungrouped nodes, leaving the existing regions in place: the model may slot
    /// a node into one of them or gather several into a new one.
    ///
    /// `None` when there is nothing useful to ask — two ungrouped nodes can form a region, and
    /// one is only worth asking about when there is a region to fold it into.
    #[must_use]
    pub fn extend(document: &Document) -> Option<Self> {
        if !can_extend(ungrouped(document).count(), document.regions().len()) {
            return None;
        }
        let regions = document.regions();
        let labels: Vec<String> = regions
            .iter()
            .enumerate()
            .map(|(index, region)| {
                let name = &region.name;
                if region.desc.is_empty() {
                    format!("[R{}] {name}", index + 1)
                } else {
                    format!("[R{}] {name} \u{2014} {}", index + 1, region.desc)
                }
            })
            .collect();
        let labelled = regions
            .iter()
            .enumerate()
            .map(|(index, region)| (format!("R{}", index + 1), region.id.clone()))
            .collect();

        let (numbered, nodes, edges) = context(document, ungrouped(document));
        let user = format!(
            "Existing regions:\n{}\n\nUngrouped nodes:\n{nodes}\n\nEdges:\n{edges}",
            if labels.is_empty() {
                "(none)".to_string()
            } else {
                labels.join("\n")
            }
        );
        Some(Self {
            system: SYSTEM_EXTEND,
            user,
            numbered,
            labelled,
            replaces: false,
        })
    }

    /// Re-partition the whole page: the answer replaces every region rather than adding to them.
    ///
    /// `None` when fewer than two nodes could be grouped at all.
    #[must_use]
    pub fn partition(document: &Document) -> Option<Self> {
        if !can_partition(groupable(document).count()) {
            return None;
        }
        let (numbered, nodes, edges) = context(document, groupable(document));
        Some(Self {
            system: SYSTEM_PARTITION,
            user: format!("Nodes:\n{nodes}\n\nEdges:\n{edges}"),
            numbered,
            labelled: HashMap::new(),
            replaces: true,
        })
    }

    #[must_use]
    pub fn system(&self) -> &'static str {
        self.system
    }

    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    /// Reads the model's reply, or `None` when nothing usable came back.
    ///
    /// Models wrap the array in prose, fences or a `<think>` block despite the instructions, so
    /// the first `[` to the last `]` is taken and parsed. A node is claimed by the first group
    /// that names it: a model that puts one node in two regions gets the first, because
    /// membership is exclusive anyway.
    #[must_use]
    pub fn parse(&self, reply: &str) -> Option<GroupingPlan> {
        let array: Vec<Value> = match serde_json::from_str(&json_array(reply)?) {
            Ok(Value::Array(items)) => items,
            _ => return None,
        };

        let mut claimed: Vec<NodeId> = Vec::new();
        let mut assignments = Vec::new();
        for item in &array {
            let members = self.members(item, &claimed);
            if members.is_empty() {
                continue;
            }
            let Some(assignment) = self.assignment(item, members) else {
                continue;
            };
            claimed.extend(assignment_members(&assignment).iter().cloned());
            assignments.push(assignment);
        }
        if assignments.is_empty() {
            return None;
        }
        Some(GroupingPlan {
            replaces: self.replaces,
            assignments,
        })
    }

    /// Geometric clustering of the same nodes the prompt described: union-find over the edges
    /// between them plus spatial proximity. No model involved — this is what happens when the
    /// one that was asked cannot answer.
    #[must_use]
    pub fn fallback(&self, document: &Document) -> GroupingPlan {
        // A full regroup replaces the regions it is counting past, so its names start at one.
        let named_from = if self.replaces {
            1
        } else {
            document.regions().len() + 1
        };
        let assignments = cluster(document, &self.numbered)
            .into_iter()
            .enumerate()
            .map(|(index, members)| Assignment::New {
                name: format!("Group {}", named_from + index),
                desc: String::new(),
                members,
            })
            .collect();
        GroupingPlan {
            replaces: self.replaces,
            assignments,
        }
    }

    /// The node ids one item names, dropping numbers that are out of range or already taken.
    fn members(&self, item: &Value, claimed: &[NodeId]) -> Vec<NodeId> {
        let mut members: Vec<NodeId> = Vec::new();
        for number in item
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(index) = number.as_u64().filter(|number| *number > 0) else {
                continue;
            };
            let Some(id) = usize::try_from(index)
                .ok()
                .and_then(|index| self.numbered.get(index - 1))
            else {
                continue;
            };
            if !claimed.contains(id) && !members.contains(id) {
                members.push(id.clone());
            }
        }
        members
    }

    /// One item as an assignment, or `None` when it is neither a known region nor a usable new
    /// one. Folding into a region can be a single node; minting one needs two.
    fn assignment(&self, item: &Value, members: Vec<NodeId>) -> Option<Assignment> {
        if let Some(region) = item
            .get("into")
            .and_then(Value::as_str)
            .and_then(|label| self.labelled.get(label.trim().to_uppercase().as_str()))
        {
            return Some(Assignment::Existing {
                region: region.clone(),
                members,
            });
        }
        let name = item.get("name").and_then(Value::as_str)?;
        if members.len() < MIN_GROUP {
            return None;
        }
        Some(Assignment::New {
            name: cut(name.trim(), MAX_NAME_CHARS),
            desc: item
                .get("desc")
                .and_then(Value::as_str)
                .map(|desc| cut(desc.trim(), MAX_DESC_CHARS))
                .unwrap_or_default(),
            members,
        })
    }
}

impl Document {
    /// Nodes on the active page that no region holds — the picker's `Ungrouped` row, and what
    /// "group ungrouped with AI" has to work with.
    #[must_use]
    pub fn ungrouped_count(&self) -> usize {
        ungrouped(self).count()
    }

    /// Nodes a grouping may touch at all.
    #[must_use]
    pub fn groupable_count(&self) -> usize {
        groupable(self).count()
    }

    /// Applies a whole grouping in one undo step, so reviewing it is one ⌘Z either way.
    ///
    /// Returns whether anything changed.
    pub fn apply_grouping(&mut self, plan: GroupingPlan) -> bool {
        if plan.assignments.is_empty() {
            return false;
        }
        self.begin(crate::history::EditKind::Structure);
        if plan.replaces {
            self.active_page_mut().regions.clear();
        }
        for assignment in plan.assignments {
            let regions = &mut self.active_page_mut().regions;
            match assignment {
                Assignment::Existing { region, members } => {
                    // A region the reply named can have been deleted since; nothing to fold into.
                    if regions.iter().any(|candidate| candidate.id == region) {
                        super::fold_into(regions, &region, members);
                    }
                }
                Assignment::New {
                    name,
                    desc,
                    members,
                } => {
                    super::insert_region(
                        regions,
                        members,
                        NewRegion {
                            name,
                            desc,
                            // The model guessed; the user reviews it.
                            status: RegionStatus::Suggested,
                        },
                    );
                }
            }
        }
        self.touch();
        true
    }
}

/// The nodes a grouping may touch: everything but freehand strokes, which are annotation
/// rather than content — `list_regions` and the picker draw the same line.
fn groupable(document: &Document) -> impl Iterator<Item = &Node> {
    document
        .nodes()
        .iter()
        .filter(|node| !matches!(node.kind, NodeKind::Draw(_)))
}

fn ungrouped(document: &Document) -> impl Iterator<Item = &Node> {
    groupable(document).filter(|node| {
        !document
            .regions()
            .iter()
            .any(|region| region.member_ids.contains(&node.id))
    })
}

/// Whether [`Prompt::extend`] has anything to ask about. Two ungrouped nodes can form a region
/// between them; one is only worth asking about when there is a region to fold it into.
///
/// Takes counts rather than the document so the command registry — which is handed a
/// [`crate::Scope`] and never the page — decides availability by the same rule the prompt does.
#[must_use]
pub const fn can_extend(ungrouped: usize, regions: usize) -> bool {
    ungrouped >= MIN_GROUP || (ungrouped > 0 && regions > 0)
}

/// Whether [`Prompt::partition`] has anything to ask about.
#[must_use]
pub const fn can_partition(groupable: usize) -> bool {
    groupable >= MIN_GROUP
}

/// The numbered node lines and the edges between them, capped at [`MAX_PROMPT_NODES`].
fn context<'a>(
    document: &Document,
    candidates: impl Iterator<Item = &'a Node>,
) -> (Vec<NodeId>, String, String) {
    let scoped: Vec<&Node> = candidates.take(MAX_PROMPT_NODES).collect();
    let numbered: Vec<NodeId> = scoped.iter().map(|node| node.id.clone()).collect();

    let nodes = scoped
        .iter()
        .enumerate()
        .map(|(index, node)| line(document, node, index + 1))
        .collect::<Vec<_>>()
        .join("\n");

    let number_of = |id: &NodeId| numbered.iter().position(|node| node == id).map(|at| at + 1);
    let edges: Vec<String> = document
        .edges()
        .iter()
        .filter_map(|edge| {
            let from = number_of(&edge.source)?;
            let to = number_of(&edge.target)?;
            Some(format!("{from}->{to}"))
        })
        .collect();

    let edges = if edges.is_empty() {
        "(none)".to_string()
    } else {
        edges.join(", ")
    };
    (numbered, nodes, edges)
}

/// `[3] query [120,-40] Active users — select * from users`.
fn line(document: &Document, node: &Node, number: usize) -> String {
    let described = describe(node, document.result(&node.id).map(AsRef::as_ref));
    let kind = node
        .node_type()
        .map_or("node", peek_document::NodeType::as_str);
    let label = described
        .as_ref()
        .map_or_else(|| kind.to_string(), |described| described.label.clone());
    let snippet = described
        .as_ref()
        .map(|described| cut(&described.snippet, MAX_SNIPPET_CHARS))
        .filter(|snippet| !snippet.is_empty())
        .map(|snippet| format!(" \u{2014} {snippet}"))
        .unwrap_or_default();
    let position = node.position;
    format!(
        "[{number}] {kind} [{},{}] {label}{snippet}",
        position.x.round(),
        position.y.round()
    )
}

/// Union-find over `candidates`: connected by an edge, or close enough on the canvas.
fn cluster(document: &Document, candidates: &[NodeId]) -> Vec<Vec<NodeId>> {
    if candidates.len() < MIN_GROUP {
        return Vec::new();
    }
    let mut roots: Vec<usize> = (0..candidates.len()).collect();
    let index_of = |id: &NodeId| candidates.iter().position(|candidate| candidate == id);

    for edge in document.edges() {
        if let (Some(from), Some(to)) = (index_of(&edge.source), index_of(&edge.target)) {
            union(&mut roots, from, to);
        }
    }

    let centres: Vec<Option<peek_document::geometry::Point>> = candidates
        .iter()
        .map(|id| document.node(id).map(|node| node.bounds().center()))
        .collect();
    for left in 0..candidates.len() {
        for right in (left + 1)..candidates.len() {
            let (Some(one), Some(other)) = (centres[left], centres[right]) else {
                continue;
            };
            if (one.x - other.x).hypot(one.y - other.y) < PROXIMITY {
                union(&mut roots, left, right);
            }
        }
    }

    let mut clusters: Vec<(usize, Vec<NodeId>)> = Vec::new();
    for (index, id) in candidates.iter().enumerate() {
        let root = find(&roots, index);
        match clusters.iter_mut().find(|(other, _)| *other == root) {
            Some((_, members)) => members.push(id.clone()),
            None => clusters.push((root, vec![id.clone()])),
        }
    }
    clusters
        .into_iter()
        .map(|(_, members)| members)
        .filter(|members| members.len() >= MIN_GROUP)
        .collect()
}

fn find(roots: &[usize], mut index: usize) -> usize {
    while roots[index] != index {
        index = roots[index];
    }
    index
}

fn union(roots: &mut [usize], left: usize, right: usize) {
    let (left, right) = (find(roots, left), find(roots, right));
    roots[left] = right;
}

fn assignment_members(assignment: &Assignment) -> &[NodeId] {
    match assignment {
        Assignment::Existing { members, .. } | Assignment::New { members, .. } => members,
    }
}

/// The first `[` to the last `]`, with any reasoning block dropped first.
fn json_array(reply: &str) -> Option<String> {
    let mut cleaned = String::with_capacity(reply.len());
    let mut rest = reply;
    while let Some(start) = rest.to_lowercase().find("<think>") {
        cleaned.push_str(&rest[..start]);
        let after = &rest[start..];
        match after.to_lowercase().find("</think>") {
            Some(end) => rest = &after[end + "</think>".len()..],
            // An unterminated block swallows the rest, which is what the reference's
            // non-greedy match does not do — but an answer inside one is not an answer.
            None => rest = "",
        }
    }
    cleaned.push_str(rest);

    let start = cleaned.find('[')?;
    let end = cleaned.rfind(']')?;
    (start < end).then(|| cleaned[start..=end].to_string())
}

fn cut(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::{Assignment, Prompt, can_extend};
    use crate::Document;
    use crate::regions::tests::{ids, suggested};
    use peek_document::geometry::{Point, Rect, Size};
    use peek_document::{
        CanvasDocument, Node, NodeId, NodeKind, NodeType, QueryData, RegionStatus, TextData,
    };

    /// A node at a position, so clustering has something to measure.
    fn node(name: &str, position: (f64, f64), kind: NodeKind) -> Node {
        let mut node = Node::new(
            NodeType::Text,
            Rect::new(Point::new(position.0, position.1), Size::new(100.0, 50.0)),
        );
        node.id = NodeId::from(name);
        node.kind = kind;
        node
    }

    fn text(name: &str, position: (f64, f64)) -> Node {
        node(
            name,
            position,
            NodeKind::Text(TextData {
                text: format!("about {name}"),
            }),
        )
    }

    fn page(nodes: Vec<Node>) -> Document {
        let mut persisted = CanvasDocument::empty();
        persisted
            .pages
            .get_mut(&persisted.active_page_id)
            .expect("the empty document has one page")
            .nodes = nodes;
        Document::load(persisted)
    }

    fn connect(document: &mut Document, from: &str, to: &str) {
        assert!(document.connect(&NodeId::from(from), &NodeId::from(to)));
    }

    fn names(document: &Document) -> Vec<(&str, Vec<NodeId>)> {
        document
            .regions()
            .iter()
            .map(|region| (region.name.as_str(), region.member_ids.clone()))
            .collect()
    }

    #[test]
    fn a_partition_prompt_numbers_every_node_and_its_edges() {
        let mut document = page(vec![text("a", (0.0, 0.0)), text("b", (900.0, 0.0))]);
        connect(&mut document, "a", "b");
        let prompt = Prompt::partition(&document).expect("two nodes are groupable");

        assert!(prompt.user().contains("[1] text [0,0] about a"));
        assert!(prompt.user().contains("[2] text [900,0] about b"));
        assert!(prompt.user().contains("Edges:\n1->2"));
    }

    /// The living-document prompt shows the regions by the `[R#]` label the reply refers back to.
    #[test]
    fn an_extend_prompt_labels_the_regions_a_node_could_join() {
        let mut document = page(vec![
            text("a", (0.0, 0.0)),
            text("b", (0.0, 0.0)),
            text("c", (0.0, 0.0)),
        ]);
        document.group_nodes(ids(&["a"]), suggested("Billing"));
        let prompt = Prompt::extend(&document).expect("two nodes are ungrouped");

        assert!(prompt.user().contains("[R1] Billing"));
        assert!(prompt.user().contains("Ungrouped nodes:\n[1] text"));
        assert!(!prompt.user().contains("about a"), "a is already grouped");
    }

    /// One ungrouped node is worth asking about only when there is a region to fold it into.
    #[test]
    fn a_lone_ungrouped_node_is_only_worth_asking_about_with_a_region_to_join() {
        assert!(!can_extend(1, 0));
        assert!(can_extend(1, 1));
        assert!(can_extend(2, 0));
    }

    /// Models wrap the array in prose, fences and a reasoning block despite the instructions.
    #[test]
    fn the_array_is_pulled_out_of_whatever_the_model_wrapped_it_in() {
        let document = page(vec![text("a", (0.0, 0.0)), text("b", (0.0, 0.0))]);
        let prompt = Prompt::partition(&document).expect("two nodes are groupable");
        let reply = "<think>hmm</think>Here you go:\n```json\n\
            [{\"name\":\"Churn Funnel\",\"desc\":\"who leaves\",\"nodes\":[1,2]}]\n```";

        let plan = prompt.parse(reply).expect("one group came back");
        assert_eq!(
            plan.assignments(),
            [Assignment::New {
                name: "Churn Funnel".to_string(),
                desc: "who leaves".to_string(),
                members: ids(&["a", "b"]),
            }]
        );
    }

    /// Membership is exclusive, so a node named twice belongs to the group that claimed it
    /// first — and a group left with one node is not a region.
    #[test]
    fn a_node_is_claimed_by_the_first_group_that_names_it() {
        let document = page(vec![
            text("a", (0.0, 0.0)),
            text("b", (0.0, 0.0)),
            text("c", (0.0, 0.0)),
        ]);
        let prompt = Prompt::partition(&document).expect("three nodes are groupable");
        let reply = r#"[{"name":"First","nodes":[1,2]},{"name":"Second","nodes":[2,3]}]"#;

        let plan = prompt.parse(reply).expect("the first group is usable");
        assert_eq!(plan.assignments().len(), 1, "the second was left with c");
    }

    /// Folding into an existing region can be a single node; minting one cannot.
    #[test]
    fn a_single_node_may_join_a_region_but_not_start_one() {
        let mut document = page(vec![
            text("a", (0.0, 0.0)),
            text("b", (0.0, 0.0)),
            text("c", (0.0, 0.0)),
        ]);
        let billing = document.group_nodes(ids(&["a"]), suggested("Billing"));
        let prompt = Prompt::extend(&document).expect("two nodes are ungrouped");
        let reply = r#"[{"into":"r1","nodes":[1]},{"name":"Alone","nodes":[2]}]"#;

        let plan = prompt.parse(reply).expect("the fold is usable");
        assert_eq!(
            plan.assignments(),
            [Assignment::Existing {
                region: billing,
                members: ids(&["b"]),
            }]
        );
    }

    #[test]
    fn a_reply_with_nothing_usable_is_refused_so_the_caller_can_fall_back() {
        let document = page(vec![text("a", (0.0, 0.0)), text("b", (0.0, 0.0))]);
        let prompt = Prompt::partition(&document).expect("two nodes are groupable");

        assert!(prompt.parse("I could not do that.").is_none());
        assert!(
            prompt
                .parse(r#"[{"name":"Ghosts","nodes":[9,10]}]"#)
                .is_none()
        );
    }

    /// A full regroup drops what was there; grouping the ungrouped keeps it.
    #[test]
    fn a_partition_replaces_the_regions_and_an_extend_adds_to_them() {
        let mut document = page(vec![
            text("a", (0.0, 0.0)),
            text("b", (0.0, 0.0)),
            text("c", (0.0, 0.0)),
        ]);
        document.group_nodes(ids(&["a"]), suggested("Billing"));

        let extend = Prompt::extend(&document).expect("two nodes are ungrouped");
        let plan = extend
            .parse(r#"[{"name":"Churn","nodes":[1,2]}]"#)
            .expect("one group");
        assert!(document.apply_grouping(plan));
        assert_eq!(
            names(&document),
            [("Billing", ids(&["a"])), ("Churn", ids(&["b", "c"]))]
        );

        let partition = Prompt::partition(&document).expect("three nodes are groupable");
        let plan = partition
            .parse(r#"[{"name":"Everything","nodes":[1,2,3]}]"#)
            .expect("one group");
        assert!(document.apply_grouping(plan));
        assert_eq!(names(&document), [("Everything", ids(&["a", "b", "c"]))]);
    }

    /// A grouping is reviewed as a whole, so undoing it is one press however many regions it
    /// touched.
    #[test]
    fn a_whole_grouping_is_one_undo_step() {
        let mut document = page(vec![
            text("a", (0.0, 0.0)),
            text("b", (0.0, 0.0)),
            text("c", (0.0, 0.0)),
            text("d", (0.0, 0.0)),
        ]);
        let prompt = Prompt::partition(&document).expect("four nodes are groupable");
        let plan = prompt
            .parse(r#"[{"name":"One","nodes":[1,2]},{"name":"Two","nodes":[3,4]}]"#)
            .expect("two groups");

        assert!(document.apply_grouping(plan));
        document.checkpoint();
        assert_eq!(document.regions().len(), 2);

        assert!(document.undo());
        assert!(document.regions().is_empty());
    }

    #[test]
    fn the_model_only_ever_suggests_so_every_region_it_made_is_reviewable() {
        let mut document = page(vec![text("a", (0.0, 0.0)), text("b", (0.0, 0.0))]);
        let prompt = Prompt::partition(&document).expect("two nodes are groupable");
        let plan = prompt
            .parse(r#"[{"name":"Churn","nodes":[1,2]}]"#)
            .expect("one group");
        document.apply_grouping(plan);

        assert_eq!(document.regions()[0].status, RegionStatus::Suggested);
    }

    /// The fallback is geometry only: an edge or 420 canvas pixels joins two nodes, and
    /// anything left alone is not a region.
    #[test]
    fn the_fallback_clusters_by_edges_and_proximity() {
        let mut document = page(vec![
            text("near-a", (0.0, 0.0)),
            text("near-b", (100.0, 0.0)),
            text("far", (5000.0, 0.0)),
            text("linked", (9000.0, 0.0)),
        ]);
        connect(&mut document, "far", "linked");
        let prompt = Prompt::partition(&document).expect("four nodes are groupable");

        let plan = prompt.fallback(&document);
        assert_eq!(
            plan.assignments(),
            [
                Assignment::New {
                    name: "Group 1".to_string(),
                    desc: String::new(),
                    members: ids(&["near-a", "near-b"]),
                },
                Assignment::New {
                    name: "Group 2".to_string(),
                    desc: String::new(),
                    members: ids(&["far", "linked"]),
                },
            ]
        );
    }

    /// Adding to a page that already has regions counts past them, so the placeholder names do
    /// not collide with the ones already on screen.
    #[test]
    fn the_fallback_names_count_past_the_regions_already_there() {
        let mut document = page(vec![
            text("a", (0.0, 0.0)),
            text("b", (0.0, 0.0)),
            text("c", (0.0, 0.0)),
        ]);
        document.group_nodes(ids(&["a"]), suggested("Billing"));
        let prompt = Prompt::extend(&document).expect("two nodes are ungrouped");

        let plan = prompt.fallback(&document);
        assert_eq!(
            plan.assignments(),
            [Assignment::New {
                name: "Group 2".to_string(),
                desc: String::new(),
                members: ids(&["b", "c"]),
            }]
        );
    }

    /// Freehand strokes are annotation, not content — they are never grouped or counted.
    #[test]
    fn drawings_are_not_groupable() {
        let document = page(vec![
            text("a", (0.0, 0.0)),
            node(
                "stroke",
                (100.0, 0.0),
                NodeKind::Draw(peek_document::DrawData::default()),
            ),
        ]);

        assert_eq!(document.groupable_count(), 1);
        assert!(Prompt::partition(&document).is_none());
    }

    /// A query node's label is its description when it has one, which is what makes the AI
    /// query labels worth having: the grouping prompt reads them.
    #[test]
    fn a_labelled_query_is_described_by_its_label() {
        let document = page(vec![
            node(
                "a",
                (0.0, 0.0),
                NodeKind::Query(QueryData {
                    query: "select count(*) from churned".to_string(),
                    description: Some("Churned Accounts".to_string()),
                    ..QueryData::default()
                }),
            ),
            text("b", (0.0, 0.0)),
        ]);
        let prompt = Prompt::partition(&document).expect("two nodes are groupable");

        assert!(prompt.user().contains("[1] query [0,0] Churned Accounts"));
    }
}
