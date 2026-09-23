//! `historyLog.ts` and the in-memory half of `historyStore.ts`: one connection's log, split
//! into verified per-page chains. Nothing here touches the disk — [`super::HistoryFile`] does.

use std::collections::BTreeMap;

use crate::ids::{CheckpointId, PageId};

use super::delta;
use super::entry::{EntryBody, HistoryEntry, PageSnapshot};

/// A full snapshot every K entries bounds both replay cost and how many deltas one corrupt
/// line can invalidate: a bad line drops the rest of its segment.
const FULL_EVERY: usize = 20;
/// A near-snapshot-sized delta ("select all, delete, rebuild") stores worse than the snapshot.
const DELTA_SIZE_RATIO: f64 = 0.6;
const MAX_ENTRIES_PER_PAGE: usize = 500;

#[derive(Debug, Clone)]
struct PageChain {
    entries: Vec<HistoryEntry>,
    tail: PageSnapshot,
    since_full: usize,
}

/// What [`ConnectionHistory::capture`] records.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub page: PageId,
    pub snapshot: PageSnapshot,
    pub taken_at: i64,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionHistory {
    chains: BTreeMap<PageId, PageChain>,
}

impl ConnectionHistory {
    /// Parses a log and caps every page at 500 entries. The flag says whether anything was
    /// trimmed, in which case the caller rewrites the file with [`Self::serialize`].
    #[must_use]
    pub fn parse(contents: &str) -> (Self, bool) {
        let entries = contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            // A truncated or hand-mangled line invalidates itself, not the log: chain
            // verification drops whatever depended on it.
            .filter_map(|line| serde_json::from_str::<HistoryEntry>(line).ok());
        let mut history = Self::default();
        for entry in entries {
            history.adopt(entry);
        }
        let compacted = history.compact();
        (history, compacted)
    }

    /// Git-style verification: a full entry always starts or continues a chain; a delta is
    /// trusted only when its parent is the chain's last entry, and anything else is dropped
    /// until the next full entry starts a fresh verified segment.
    fn adopt(&mut self, entry: HistoryEntry) {
        match &entry.body {
            EntryBody::Full { snapshot } => {
                let tail = snapshot.clone();
                let chain = self
                    .chains
                    .entry(entry.page_id.clone())
                    .or_insert_with(|| PageChain {
                        entries: Vec::new(),
                        tail: tail.clone(),
                        since_full: 0,
                    });
                chain.tail = tail;
                chain.since_full = 0;
                chain.entries.push(entry);
            }
            EntryBody::Delta { delta: page_delta } => {
                let Some(chain) = self.chains.get_mut(&entry.page_id) else {
                    return;
                };
                if chain.entries.last().map(|last| &last.id) != entry.parent_id.as_ref() {
                    return;
                }
                let tail = std::mem::replace(&mut chain.tail, PageSnapshot::empty(""));
                chain.tail = delta::apply(tail, page_delta);
                chain.since_full += 1;
                chain.entries.push(entry);
            }
        }
    }

    /// Cuts each over-long chain on a full-snapshot boundary, so every kept delta still has
    /// its base.
    fn compact(&mut self) -> bool {
        let mut trimmed = false;
        for chain in self.chains.values_mut() {
            let length = chain.entries.len();
            if length <= MAX_ENTRIES_PER_PAGE {
                continue;
            }
            let cutoff = length - MAX_ENTRIES_PER_PAGE;
            let start = chain
                .entries
                .iter()
                .enumerate()
                .position(|(index, entry)| index >= cutoff && entry.is_full());
            let Some(start) = start.filter(|start| *start > 0) else {
                continue;
            };
            chain.entries.drain(..start);
            trimmed = true;
        }
        trimmed
    }

    /// The whole log, one line per entry and a trailing newline — what a compaction writes.
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut contents = String::new();
        for entry in self.chains.values().flat_map(|chain| &chain.entries) {
            if let Ok(line) = serde_json::to_string(entry) {
                contents.push_str(&line);
                contents.push('\n');
            }
        }
        contents
    }

    /// Verified entries for one page, oldest first.
    #[must_use]
    pub fn entries(&self, page: &PageId) -> &[HistoryEntry] {
        self.chains
            .get(page)
            .map_or(&[], |chain| chain.entries.as_slice())
    }

    /// The page as it was at `entry`: the nearest full snapshot at or before it, then the
    /// deltas after it replayed forward.
    #[must_use]
    pub fn reconstruct(&self, page: &PageId, entry: &CheckpointId) -> Option<PageSnapshot> {
        let entries = &self.chains.get(page)?.entries;
        let index = entries
            .iter()
            .position(|candidate| &candidate.id == entry)?;
        let base = entries[..=index].iter().rposition(HistoryEntry::is_full)?;
        let EntryBody::Full { snapshot } = &entries[base].body else {
            return None;
        };
        let replayed =
            entries[base + 1..=index]
                .iter()
                .fold(snapshot.clone(), |snapshot, entry| match &entry.body {
                    EntryBody::Delta { delta: page_delta } => delta::apply(snapshot, page_delta),
                    EntryBody::Full { snapshot } => snapshot.clone(),
                });
        Some(replayed)
    }

    /// Appends a checkpoint unless the page matches its chain's tail, and returns the entry to
    /// write. Diffing against the tail rebuilt from the log rather than the last autosave is
    /// what lets a chain heal after a crash.
    pub fn capture(&mut self, checkpoint: Checkpoint) -> Option<&HistoryEntry> {
        let Checkpoint {
            page,
            snapshot,
            taken_at,
            label,
        } = checkpoint;
        let chain = self.chains.get(&page);
        if chain.is_some_and(|chain| chain.tail == snapshot) {
            return None;
        }
        let empty = PageSnapshot::empty(snapshot.name.clone());
        let (page_delta, summary) =
            delta::diff(chain.map_or(&empty, |chain| &chain.tail), &snapshot);
        let parent = chain.and_then(|chain| chain.entries.last());
        let as_full = chain.is_none_or(|chain| chain.since_full + 1 >= FULL_EVERY)
            || delta_outweighs_snapshot(&page_delta, &snapshot);
        let entry = HistoryEntry {
            id: CheckpointId::generate(),
            parent_id: parent.map(|parent| parent.id.clone()),
            page_id: page.clone(),
            seq: parent.map_or(0, |parent| parent.seq) + 1,
            taken_at,
            label,
            summary,
            body: if as_full {
                EntryBody::Full {
                    snapshot: snapshot.clone(),
                }
            } else {
                EntryBody::Delta { delta: page_delta }
            },
        };
        let chain = self.chains.entry(page).or_insert_with(|| PageChain {
            entries: Vec::new(),
            tail: snapshot.clone(),
            since_full: 0,
        });
        chain.since_full = if as_full { 0 } else { chain.since_full + 1 };
        chain.tail = snapshot;
        chain.entries.push(entry);
        chain.entries.last()
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "byte counts compared by ratio; precision past 2^52 bytes is irrelevant"
)]
fn delta_outweighs_snapshot(page_delta: &super::PageDelta, snapshot: &PageSnapshot) -> bool {
    json_length(page_delta) as f64 > json_length(snapshot) as f64 * DELTA_SIZE_RATIO
}

fn json_length(value: &impl serde::Serialize) -> usize {
    serde_json::to_string(value).map_or(0, |json| json.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;
    use crate::ids::NodeId;
    use crate::node::{Node, NodeKind, TextData};

    fn text(id: &str, body: &str) -> Node {
        Node {
            id: NodeId::from(id),
            position: Point::new(0.0, 0.0),
            width: None,
            height: None,
            measured: None,
            selected: false,
            kind: NodeKind::Text(TextData {
                text: body.to_string(),
            }),
        }
    }

    /// A page of ten notes, so a one-note edit is a delta rather than outweighing a snapshot.
    fn page_with(edit: &str) -> PageSnapshot {
        let mut nodes: Vec<Node> = (0..10)
            .map(|index| {
                text(
                    &format!("note_{index}"),
                    "a long enough body to outweigh a delta",
                )
            })
            .collect();
        nodes[0] = text("note_0", edit);
        PageSnapshot {
            nodes,
            ..PageSnapshot::empty("Page")
        }
    }

    fn checkpoint(snapshot: PageSnapshot) -> Checkpoint {
        Checkpoint {
            page: PageId::from("page_1"),
            snapshot,
            taken_at: 1,
            label: None,
        }
    }

    fn history_of(edits: usize) -> ConnectionHistory {
        let mut history = ConnectionHistory::default();
        for edit in 0..edits {
            history.capture(checkpoint(page_with(&edit.to_string())));
        }
        history
    }

    #[test]
    fn the_first_capture_is_full_and_the_next_are_deltas() {
        let history = history_of(3);
        let entries = history.entries(&PageId::from("page_1"));
        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_full());
        assert!(!entries[1].is_full());
        assert_eq!(entries[2].seq, 3);
        assert_eq!(entries[2].parent_id.as_ref(), Some(&entries[1].id));
    }

    #[test]
    fn capturing_an_unchanged_page_records_nothing() {
        let mut history = history_of(1);
        assert!(history.capture(checkpoint(page_with("0"))).is_none());
        assert_eq!(history.entries(&PageId::from("page_1")).len(), 1);
    }

    #[test]
    fn every_twentieth_entry_is_full() {
        let history = history_of(21);
        let entries = history.entries(&PageId::from("page_1"));
        let full: Vec<u32> = entries
            .iter()
            .filter(|entry| entry.is_full())
            .map(|entry| entry.seq)
            .collect();
        assert_eq!(full, vec![1, 21]);
    }

    #[test]
    fn a_delta_that_outweighs_the_snapshot_is_stored_full() {
        let mut history = history_of(1);
        let replaced = PageSnapshot {
            nodes: vec![text("other", "entirely new")],
            ..PageSnapshot::empty("Page")
        };
        let entry = history.capture(checkpoint(replaced)).unwrap();
        assert!(entry.is_full());
    }

    #[test]
    fn a_round_trip_through_the_log_reconstructs_every_version() {
        let history = history_of(5);
        let (reloaded, compacted) = ConnectionHistory::parse(&history.serialize());
        assert!(!compacted);
        let page = PageId::from("page_1");
        for (index, entry) in reloaded.entries(&page).iter().enumerate() {
            assert_eq!(
                reloaded.reconstruct(&page, &entry.id),
                Some(page_with(&index.to_string()))
            );
        }
    }

    #[test]
    fn a_broken_link_drops_deltas_until_the_next_full_entry() {
        let history = history_of(22);
        let mut lines: Vec<&str> = Vec::new();
        let serialized = history.serialize();
        lines.extend(serialized.lines());
        // Drop version 3: versions 4..=20 lose their parent, version 21 is full again.
        lines.remove(2);
        let (reloaded, _) = ConnectionHistory::parse(&lines.join("\n"));
        let seqs: Vec<u32> = reloaded
            .entries(&PageId::from("page_1"))
            .iter()
            .map(|entry| entry.seq)
            .collect();
        assert_eq!(seqs, vec![1, 2, 21, 22]);
    }

    #[test]
    fn a_garbled_line_is_skipped() {
        let history = history_of(2);
        let contents = format!("{{not json\n{}", history.serialize());
        let (reloaded, _) = ConnectionHistory::parse(&contents);
        assert_eq!(reloaded.entries(&PageId::from("page_1")).len(), 2);
    }

    #[test]
    fn compaction_cuts_on_a_full_boundary() {
        let history = history_of(530);
        let (reloaded, compacted) = ConnectionHistory::parse(&history.serialize());
        assert!(compacted);
        let entries = reloaded.entries(&PageId::from("page_1"));
        // 530 - 500 = 30; the first full entry at or after index 30 is version 41.
        assert_eq!(entries[0].seq, 41);
        assert!(entries[0].is_full());
        assert_eq!(entries.len(), 490);
    }
}
