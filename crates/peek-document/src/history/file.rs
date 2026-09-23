use std::io::Write;
use std::path::{Path, PathBuf};

use peek_config::PersistenceMode;

use crate::storage::StorageError;

/// A handle to one connection's `<connection>.history.jsonl`.
///
/// Every write is refused in read-only mode; reads are not, so a read-only run can still
/// scrub the versions the TypeScript app recorded.
#[derive(Debug, Clone)]
pub struct HistoryFile {
    path: PathBuf,
    mode: PersistenceMode,
}

impl HistoryFile {
    pub(crate) fn new(path: PathBuf, mode: PersistenceMode) -> Self {
        Self { path, mode }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The log's contents, or an empty string when no version has been recorded yet.
    ///
    /// # Errors
    /// Returns an error when the file exists but cannot be read.
    pub fn load(&self) -> Result<String, StorageError> {
        if !self.path.exists() {
            return Ok(String::new());
        }
        Ok(std::fs::read_to_string(&self.path)?)
    }

    /// Appends one entry. Appending rather than rewriting is what lets both apps record into
    /// the same log without either clobbering the other's lines.
    ///
    /// # Errors
    /// [`StorageError::ReadOnly`] in read-only mode, or an io error.
    pub fn append(&self, line: &str) -> Result<(), StorageError> {
        self.writable()?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Replaces the whole log. Only compaction does this.
    ///
    /// # Errors
    /// [`StorageError::ReadOnly`] in read-only mode, or an io error.
    pub fn rewrite(&self, contents: &str) -> Result<(), StorageError> {
        self.writable()?;
        std::fs::write(&self.path, contents)?;
        Ok(())
    }

    fn writable(&self) -> Result<(), StorageError> {
        if !self.mode.can_write() {
            return Err(StorageError::ReadOnly);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("peek-history-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn appends_lines_and_reads_them_back() {
        let dir = scratch("append");
        let file = HistoryFile::new(dir.join("local.history.jsonl"), PersistenceMode::ReadWrite);
        assert_eq!(file.load().unwrap(), "");
        file.append("{\"a\":1}").unwrap();
        file.append("{\"b\":2}").unwrap();
        assert_eq!(file.load().unwrap(), "{\"a\":1}\n{\"b\":2}\n");
        file.rewrite("{\"b\":2}\n").unwrap();
        assert_eq!(file.load().unwrap(), "{\"b\":2}\n");
    }

    #[test]
    fn a_read_only_run_never_touches_disk() {
        let dir = scratch("read-only");
        let file = HistoryFile::new(dir.join("local.history.jsonl"), PersistenceMode::ReadOnly);
        assert!(matches!(file.append("{}"), Err(StorageError::ReadOnly)));
        assert!(matches!(file.rewrite(""), Err(StorageError::ReadOnly)));
        assert!(!dir.exists());
    }
}
