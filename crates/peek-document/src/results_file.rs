//! The rows sidecar, `~/peek/workspaces/<workspace>/<connection>.results.json`.
//!
//! Rows are deliberately not in the document: on this machine the documents are tens of
//! kilobytes and their sidecars run to 13 MB, so inlining them would make every autosave rewrite
//! megabytes and every undo snapshot copy them. `~/labs/peek/src/canvas/hooks/useLoadDocument.ts`
//! lifts rows out of any pre-sidecar document it still finds (see [`ResultData::legacy_rows`]).
//!
//! Shape: a flat `{ "<result node id>": [[[name, value, type], …], …] }`, which
//! `useAutoSaveResults.ts` writes as a plain `JSON.stringify` of its results map.
//!
//! [`ResultData::legacy_rows`]: crate::ResultData::legacy_rows

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use peek_config::PersistenceMode;
use serde_json::Value;

use crate::ids::NodeId;
use crate::result::ResultSet;
use crate::storage::StorageError;

/// One connection's rows, keyed by result node id.
///
/// The sets are behind an [`Arc`] because the canvas hands one to its result node's table on
/// every frame: a 6,515-row result copied per frame per visible node is tens of thousands of
/// allocations for rows that almost never change. Sharing also lets the table decide whether
/// it has new rows with a pointer comparison instead of walking every cell.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultSidecar {
    sets: BTreeMap<NodeId, Arc<ResultSet>>,
}

impl ResultSidecar {
    #[must_use]
    pub fn get(&self, node: &NodeId) -> Option<&Arc<ResultSet>> {
        self.sets.get(node)
    }

    pub fn insert(&mut self, node: NodeId, set: impl Into<Arc<ResultSet>>) {
        self.sets.insert(node, set.into());
    }

    pub fn remove(&mut self, node: &NodeId) -> Option<Arc<ResultSet>> {
        self.sets.remove(node)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    pub fn node_ids(&self) -> impl Iterator<Item = &NodeId> {
        self.sets.keys()
    }

    /// Drops rows for results the document no longer contains, so a sidecar does not grow
    /// forever as result nodes come and go.
    pub fn retain_nodes(&mut self, live: &[NodeId]) {
        self.sets.retain(|id, _| live.contains(id));
    }

    /// Parses the sidecar. A malformed entry is skipped rather than failing the load: this file
    /// is a cache, and a broken one must never stop a document from opening.
    #[must_use]
    pub fn from_json(contents: &str) -> Self {
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(contents) else {
            return Self::default();
        };
        let sets = map
            .into_iter()
            .map(|(id, rows)| {
                (
                    NodeId::from(id.as_str()),
                    Arc::new(ResultSet::from_sidecar_rows(&rows)),
                )
            })
            .collect();
        Self { sets }
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        let map: serde_json::Map<String, Value> = self
            .sets
            .iter()
            .map(|(id, set)| (id.to_string(), set.to_sidecar_rows()))
            .collect();
        Value::Object(map).to_string()
    }
}

/// A handle to one connection's sidecar, gated and guarded exactly as [`crate::DocumentFile`] is.
///
/// It keeps no `.bak`: the sidecar is reproducible by re-running the queries, so a backup of it
/// would cost megabytes to protect nothing.
#[derive(Debug)]
pub struct ResultsFile {
    path: PathBuf,
    mode: PersistenceMode,
    seen: Option<SystemTime>,
}

impl ResultsFile {
    pub(crate) fn new(path: PathBuf, mode: PersistenceMode) -> Self {
        Self {
            path,
            mode,
            seen: None,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the sidecar, yielding an empty one when it does not exist or cannot be parsed.
    ///
    /// # Errors
    /// Returns an io error only when the file exists and cannot be read at all.
    pub fn load(&mut self) -> Result<ResultSidecar, StorageError> {
        if !self.path.exists() {
            return Ok(ResultSidecar::default());
        }
        let contents = std::fs::read_to_string(&self.path)?;
        self.seen = std::fs::metadata(&self.path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        Ok(ResultSidecar::from_json(&contents))
    }

    /// # Errors
    /// [`StorageError::ReadOnly`] in read-only mode, [`StorageError::ChangedOnDisk`] when
    /// another writer got there first, or an io error.
    pub fn save(&mut self, sidecar: &ResultSidecar) -> Result<(), StorageError> {
        if !self.mode.can_write() {
            log::info!("peek: read-only mode, not saving {}", self.path.display());
            return Err(StorageError::ReadOnly);
        }
        let current = std::fs::metadata(&self.path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        if self.path.exists() && current != self.seen {
            return Err(StorageError::ChangedOnDisk);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, sidecar.to_json())?;
        std::fs::rename(&temp, &self.path)?;
        self.seen = std::fs::metadata(&self.path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ResultSidecar, ResultsFile};
    use crate::ids::NodeId;
    use crate::result::{Cell, Column, ResultSet};
    use peek_config::PersistenceMode;

    fn set() -> ResultSet {
        ResultSet::new(
            vec![Column::new("id", "INT4")],
            vec![vec![Cell::Int(1)], vec![Cell::Int(2)]],
        )
    }

    fn node() -> NodeId {
        NodeId::from("query_abc12345-result-0")
    }

    /// Its own directory per test: these run in parallel and a shared temp filename makes the
    /// mtime guard fire against another test's write.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("peek-rs-results-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("conn.results.json")
    }

    #[test]
    fn round_trips_through_the_sidecar_shape() {
        let mut sidecar = ResultSidecar::default();
        sidecar.insert(node(), set());
        let json = sidecar.to_json();
        assert!(
            json.starts_with(r#"{"query_abc12345-result-0":[[["id",1,"INT4"]]"#),
            "{json}"
        );
        assert_eq!(ResultSidecar::from_json(&json), sidecar);
    }

    /// The sidecar is a cache. A corrupt one must not be able to stop a document opening.
    #[test]
    fn a_broken_sidecar_loads_as_empty() {
        for broken in ["", "not json", "[]", "null", r#"{"a": 3}"#] {
            assert!(ResultSidecar::from_json(broken).get(&node()).is_none());
        }
    }

    #[test]
    fn retain_drops_rows_for_deleted_results() {
        let mut sidecar = ResultSidecar::default();
        sidecar.insert(node(), set());
        sidecar.insert(NodeId::from("gone-result-0"), set());
        sidecar.retain_nodes(&[node()]);
        assert_eq!(sidecar.len(), 1);
        assert!(sidecar.get(&node()).is_some());
    }

    #[test]
    fn a_read_only_handle_never_writes() {
        let path = scratch("readonly");
        let mut file = ResultsFile::new(path.clone(), PersistenceMode::ReadOnly);
        assert!(file.save(&ResultSidecar::default()).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn save_is_atomic_and_round_trips() {
        let path = scratch("write");
        let mut file = ResultsFile::new(path.clone(), PersistenceMode::ReadWrite);
        let mut sidecar = ResultSidecar::default();
        sidecar.insert(node(), set());
        file.save(&sidecar).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(file.load().unwrap(), sidecar);
    }

    /// The Tauri app autosaves the same file every three seconds; refusing beats clobbering.
    #[test]
    fn save_refuses_when_the_file_changed_under_us() {
        let path = scratch("conflict");
        let mut file = ResultsFile::new(path.clone(), PersistenceMode::ReadWrite);
        file.save(&ResultSidecar::default()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&path, r#"{"other":[]}"#).unwrap();
        assert!(file.save(&ResultSidecar::default()).is_err());
    }
}
