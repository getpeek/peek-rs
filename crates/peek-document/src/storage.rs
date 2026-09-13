//! File layout under `~/peek` (ported from the Tauri host's `storage_commands.rs`):
//! `workspaces/<workspace>/<connection>.json` for documents, `.results.json` for the rows
//! sidecar. Names are lowercased for the path.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use peek_config::PersistenceMode;

use crate::document::{CanvasDocument, DocumentError};
use crate::results_file::ResultsFile;

#[derive(Debug)]
pub enum StorageError {
    Config(peek_config::ConfigError),
    ReadOnly,
    /// Someone else — almost always the Tauri app, which autosaves the same files — wrote the
    /// document since this handle last read or wrote it. Refusing beats last-writer-wins.
    ChangedOnDisk,
    Io(std::io::Error),
    Document(DocumentError),
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(formatter, "{error}"),
            Self::ReadOnly => write!(formatter, "documents are read-only in this build"),
            Self::ChangedOnDisk => {
                write!(
                    formatter,
                    "the document changed on disk since it was loaded"
                )
            }
            Self::Io(error) => write!(formatter, "document io error: {error}"),
            Self::Document(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone)]
pub struct DocumentStore {
    base: PathBuf,
    mode: PersistenceMode,
}

impl DocumentStore {
    /// A store rooted at `~/peek`.
    ///
    /// # Errors
    /// Returns an error when the config directory cannot be resolved.
    pub fn new(mode: PersistenceMode) -> Result<Self, StorageError> {
        let base = peek_config::config_dir().map_err(StorageError::Config)?;
        Ok(Self::with_base(base, mode))
    }

    #[must_use]
    pub fn with_base(base: PathBuf, mode: PersistenceMode) -> Self {
        Self { base, mode }
    }

    #[must_use]
    pub fn mode(&self) -> PersistenceMode {
        self.mode
    }

    /// Opens a handle to one connection's document.
    ///
    /// # Errors
    /// Returns an error when the workspace directory cannot be resolved or created.
    pub fn open(&self, workspace: &str, connection: &str) -> Result<DocumentFile, StorageError> {
        Ok(DocumentFile {
            path: self.document_path(workspace, connection)?,
            mode: self.mode,
            seen: None,
            backed_up: false,
        })
    }

    /// Opens a handle to one connection's rows sidecar, beside its document.
    ///
    /// # Errors
    /// Returns an error when the workspace directory cannot be resolved or created.
    pub fn open_results(
        &self,
        workspace: &str,
        connection: &str,
    ) -> Result<ResultsFile, StorageError> {
        let dir = self.workspace_dir(workspace)?;
        let path = dir.join(format!("{}.results.json", connection.to_lowercase()));
        Ok(ResultsFile::new(path, self.mode))
    }

    fn document_path(&self, workspace: &str, connection: &str) -> Result<PathBuf, StorageError> {
        let dir = self.workspace_dir(workspace)?;
        Ok(dir.join(format!("{}.json", connection.to_lowercase())))
    }

    /// Resolves a workspace's directory. In read-only mode nothing is created or moved; the
    /// legacy flat layout is still *read* so old installs open.
    fn workspace_dir(&self, workspace: &str) -> Result<PathBuf, StorageError> {
        let workspace = workspace.to_lowercase();
        if self.mode.can_write() {
            return migrate_workspace_dir(&self.base, &workspace);
        }
        let new_dir = self.base.join("workspaces").join(&workspace);
        if new_dir.exists() {
            return Ok(new_dir);
        }
        Ok(self.base.join(&workspace))
    }
}

/// A handle to one connection's document file.
///
/// Writes are atomic (temp file plus rename) because autosave runs every few seconds against
/// files the TypeScript app may also be writing, and they are guarded on the modification time
/// this handle last saw, so a concurrent edit is refused rather than overwritten.
#[derive(Debug)]
pub struct DocumentFile {
    path: PathBuf,
    mode: PersistenceMode,
    seen: Option<SystemTime>,
    backed_up: bool,
}

impl DocumentFile {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `None` when no document exists yet, or when it is the Tauri host's `"{}"` sentinel.
    ///
    /// # Errors
    /// Returns an error when the file exists but cannot be read or parsed.
    pub fn load(&mut self) -> Result<Option<CanvasDocument>, StorageError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&self.path)?;
        self.seen = modified(&self.path);
        if contents.trim() == "{}" {
            return Ok(None);
        }
        CanvasDocument::from_json(&contents)
            .map(Some)
            .map_err(StorageError::Document)
    }

    /// # Errors
    /// [`StorageError::ReadOnly`] in read-only mode, [`StorageError::ChangedOnDisk`] when
    /// another writer got there first, or an io error.
    pub fn save(&mut self, document: &CanvasDocument) -> Result<(), StorageError> {
        if !self.mode.can_write() {
            log::info!("peek: read-only mode, not saving {}", self.path.display());
            return Err(StorageError::ReadOnly);
        }
        let current = modified(&self.path);
        if self.path.exists() && current != self.seen {
            return Err(StorageError::ChangedOnDisk);
        }
        self.back_up()?;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, document.to_json())?;
        std::fs::rename(&temp, &self.path)?;
        self.seen = modified(&self.path);
        Ok(())
    }

    /// One `<connection>.json.bak` per session, written before the first save only, so a bad
    /// first write while the TypeScript app is running is recoverable by renaming a file.
    fn back_up(&mut self) -> Result<(), StorageError> {
        if self.backed_up {
            return Ok(());
        }
        self.backed_up = true;
        if !self.path.exists() {
            return Ok(());
        }
        std::fs::copy(&self.path, self.path.with_extension("json.bak"))?;
        Ok(())
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Silently migrates the pre-3.x flat layout (`~/peek/{workspace}`) into
/// `~/peek/workspaces/{workspace}` the first time it is accessed.
fn migrate_workspace_dir(base: &Path, workspace: &str) -> Result<PathBuf, StorageError> {
    let new_dir = base.join("workspaces").join(workspace);
    if new_dir.exists() {
        return Ok(new_dir);
    }

    let legacy_dir = base.join(workspace);
    // Guard the one colliding name: a workspace literally called "workspaces" maps its
    // legacy dir onto the new container itself — never move that.
    let is_container = new_dir.parent() == Some(legacy_dir.as_path());
    if !is_container && legacy_dir.exists() {
        let container = new_dir.parent().expect("workspaces dir has a parent");
        std::fs::create_dir_all(container)?;
        std::fs::rename(&legacy_dir, &new_dir)?;
        return Ok(new_dir);
    }

    std::fs::create_dir_all(&new_dir)?;
    Ok(new_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("peek-rs-migrate-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn migrates_legacy_dir_and_preserves_files() {
        let base = scratch("legacy").join("peek");
        let legacy = base.join("prod");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("maindb.json"), "{\"doc\":1}").unwrap();

        let resolved = migrate_workspace_dir(&base, "prod").unwrap();

        assert_eq!(resolved, base.join("workspaces").join("prod"));
        assert!(
            !legacy.exists(),
            "legacy dir should be moved, not left behind"
        );
        assert_eq!(
            std::fs::read_to_string(resolved.join("maindb.json")).unwrap(),
            "{\"doc\":1}"
        );
    }

    #[test]
    fn keeps_new_dir_and_ignores_stale_legacy() {
        let base = scratch("already").join("peek");
        let new = base.join("workspaces").join("prod");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("maindb.json"), "new").unwrap();
        let legacy = base.join("prod");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("maindb.json"), "stale").unwrap();

        let resolved = migrate_workspace_dir(&base, "prod").unwrap();

        assert_eq!(resolved, new);
        assert_eq!(
            std::fs::read_to_string(new.join("maindb.json")).unwrap(),
            "new"
        );
    }

    #[test]
    fn creates_fresh_dir_when_nothing_exists() {
        let base = scratch("fresh").join("peek");
        let resolved = migrate_workspace_dir(&base, "brandnew").unwrap();
        assert_eq!(resolved, base.join("workspaces").join("brandnew"));
        assert!(resolved.is_dir());
    }

    #[test]
    fn does_not_clobber_container_for_workspace_named_workspaces() {
        let base = scratch("collide").join("peek");
        let container = base.join("workspaces");
        std::fs::create_dir_all(&container).unwrap();
        std::fs::write(container.join("other.json"), "sibling").unwrap();

        let resolved = migrate_workspace_dir(&base, "workspaces").unwrap();

        assert_eq!(resolved, container.join("workspaces"));
        assert_eq!(
            std::fs::read_to_string(container.join("other.json")).unwrap(),
            "sibling"
        );
    }

    fn writable(tag: &str) -> (PathBuf, DocumentFile) {
        let base = scratch(tag).join("peek");
        let store = DocumentStore::with_base(base.clone(), PersistenceMode::ReadWrite);
        let file = store.open("prod", "maindb").unwrap();
        (base, file)
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_file() {
        let (_base, mut file) = writable("atomic");
        file.save(&CanvasDocument::empty()).unwrap();

        assert!(file.path().exists());
        assert!(!file.path().with_extension("json.tmp").exists());
        let reloaded = file.load().unwrap().unwrap();
        assert_eq!(reloaded.pages.len(), 1);
    }

    #[test]
    fn save_refuses_when_the_file_changed_under_us() {
        let (_base, mut file) = writable("conflict");
        file.save(&CanvasDocument::empty()).unwrap();

        // Stand in for the TypeScript app autosaving the same document.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(file.path(), CanvasDocument::empty().to_json()).unwrap();

        assert!(matches!(
            file.save(&CanvasDocument::empty()),
            Err(StorageError::ChangedOnDisk)
        ));
    }

    #[test]
    fn first_save_writes_a_backup_of_the_previous_contents() {
        let (_base, mut file) = writable("backup");
        std::fs::create_dir_all(file.path().parent().unwrap()).unwrap();
        std::fs::write(file.path(), "{\"version\":1,\"before\":true}").unwrap();
        file.seen = modified(file.path());

        file.save(&CanvasDocument::empty()).unwrap();
        let backup = file.path().with_extension("json.bak");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "{\"version\":1,\"before\":true}"
        );

        // Only once per session: a second save must not overwrite the original backup.
        file.save(&CanvasDocument::empty()).unwrap();
        assert!(std::fs::read_to_string(&backup).unwrap().contains("before"));
    }

    #[test]
    fn read_only_store_never_touches_disk() {
        let base = scratch("readonly").join("peek");
        let legacy = base.join("prod");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("maindb.json"),
            CanvasDocument::empty().to_json(),
        )
        .unwrap();

        let store = DocumentStore::with_base(base.clone(), PersistenceMode::ReadOnly);
        let mut file = store.open("Prod", "MainDB").unwrap();
        let loaded = file.load().unwrap();
        assert!(loaded.is_some(), "legacy layout is still readable");
        assert!(legacy.exists(), "read-only mode must not migrate");
        assert!(!base.join("workspaces").exists());
        assert!(matches!(
            file.save(&CanvasDocument::empty()),
            Err(StorageError::ReadOnly)
        ));
    }
}
