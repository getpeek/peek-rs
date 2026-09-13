//! Debounced document persistence, ported from `~/labs/peek/src/canvas/hooks/useAutoSaveDocument.ts`.
//!
//! Every mutation bumps the session document's revision; three seconds after the last one the
//! document is written. Selection changes deliberately do not bump the revision, so moving the
//! selection around never schedules a write.

use std::time::Duration;

use gpui_kit::{App, Context, Entity, Subscription, Task};
use peek_canvas::Document;
use peek_document::{DocumentFile, ResultsFile, StorageError};

/// Matches the TypeScript hook: restart on every edit, with no maximum wait.
const DEBOUNCE: Duration = Duration::from_secs(3);

pub(crate) struct Autosave {
    document: Entity<Document>,
    file: DocumentFile,
    /// The rows sidecar, written on the same debounce but tracked separately: a query changes
    /// the rows and not the document, and an edit changes the document and not the rows.
    ///
    /// Unlike `useAutoSaveResults.ts`, this one is flushed on quit. The reference flushes the
    /// document but not the results, so the last query before a connection switch is lost.
    results_file: Option<ResultsFile>,
    saved_results_revision: u64,
    results_stale: bool,
    saved_revision: u64,
    /// Dropping the task cancels it, so reassigning *is* restart-the-debounce.
    pending: Option<Task<()>>,
    /// Set once the file has been changed by someone else; we stop writing rather than
    /// clobber whatever the other writer put there.
    stale: bool,
    _observer: Subscription,
    _quit: Subscription,
}

impl std::fmt::Debug for Autosave {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Autosave")
            .field("path", &self.file.path())
            .field("saved_revision", &self.saved_revision)
            .field("stale", &self.stale)
            .finish_non_exhaustive()
    }
}

impl Autosave {
    pub(crate) fn new(
        document: Entity<Document>,
        files: (DocumentFile, Option<ResultsFile>),
        cx: &mut Context<Self>,
    ) -> Self {
        let (file, results_file) = files;
        let saved_revision = document.read(cx).revision();
        let saved_results_revision = document.read(cx).results_revision();
        Self {
            _observer: cx.observe(&document, |this, _, cx| this.arm(cx)),
            _quit: cx.on_app_quit(|this, cx| {
                this.flush(cx);
                async {}
            }),
            document,
            file,
            results_file,
            saved_results_revision,
            results_stale: false,
            saved_revision,
            pending: None,
            stale: false,
        }
    }

    /// Restarts the debounce, unless nothing persisted has actually changed.
    fn arm(&mut self, cx: &mut Context<Self>) {
        if !self.document_pending(cx) && !self.results_pending(cx) {
            return;
        }
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, Autosave::flush).ok();
        }));
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.document.read(cx).revision() != self.saved_revision
    }

    fn document_pending(&self, cx: &App) -> bool {
        !self.stale && self.is_dirty(cx)
    }

    fn results_pending(&self, cx: &App) -> bool {
        !self.results_stale
            && self.results_file.is_some()
            && self.document.read(cx).results_revision() != self.saved_results_revision
    }

    pub(crate) fn flush(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        self.flush_document(cx);
        self.flush_results(cx);
    }

    fn flush_document(&mut self, cx: &mut Context<Self>) {
        if !self.document_pending(cx) {
            return;
        }
        let document = self.document.read(cx);
        let revision = document.revision();
        match self.file.save(document.inner()) {
            Ok(()) => self.saved_revision = revision,
            Err(StorageError::ChangedOnDisk) => {
                self.stale = true;
                log::error!(
                    "peek: {} changed on disk; edits in this window are no longer being saved",
                    self.file.path().display()
                );
            }
            Err(error) => log::error!("peek: could not save the document: {error}"),
        }
    }

    fn flush_results(&mut self, cx: &mut Context<Self>) {
        if !self.results_pending(cx) {
            return;
        }
        let document = self.document.read(cx);
        let revision = document.results_revision();
        let rows = document.results().clone();
        let Some(file) = self.results_file.as_mut() else {
            return;
        };
        match file.save(&rows) {
            Ok(()) => self.saved_results_revision = revision,
            Err(StorageError::ChangedOnDisk) => {
                self.results_stale = true;
                log::error!(
                    "peek: {} changed on disk; results in this window are no longer being saved",
                    file.path().display()
                );
            }
            Err(error) => log::error!("peek: could not save results: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, TestAppContext};
    use peek_canvas::{Point, Rect, Size};
    use peek_config::PersistenceMode;
    use peek_document::{CanvasDocument, DocumentStore, NodeType};

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("peek-rs-autosave-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn harness(
        tag: &str,
        mode: PersistenceMode,
        cx: &mut TestAppContext,
    ) -> (std::path::PathBuf, Entity<Document>, Entity<Autosave>) {
        let base = scratch(tag).join("peek");
        let store = DocumentStore::with_base(base, mode);
        let file = store.open("prod", "maindb").unwrap();
        let path = file.path().to_path_buf();
        let (document, autosave) = cx.update(|cx| {
            let document = cx.new(|_| Document::load(CanvasDocument::empty()));
            let autosave = cx.new(|cx| Autosave::new(document.clone(), (file, None), cx));
            (document, autosave)
        });
        (path, document, autosave)
    }

    fn add_node(document: &Entity<Document>, cx: &mut TestAppContext) {
        document.update(cx, |document, cx| {
            document.create_node(
                NodeType::Text,
                Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 100.0)),
            );
            cx.notify();
        });
    }

    #[gpui_kit::test]
    fn writes_once_three_seconds_after_the_last_edit(cx: &mut TestAppContext) {
        let (path, document, _autosave) = harness("debounce", PersistenceMode::ReadWrite, cx);

        add_node(&document, cx);
        cx.run_until_parked();
        assert!(!path.exists(), "nothing is written before the debounce");

        cx.executor()
            .advance_clock(DEBOUNCE + Duration::from_millis(100));
        cx.run_until_parked();

        let written = CanvasDocument::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written.active_page().unwrap().nodes.len(), 1);
    }

    #[gpui_kit::test]
    fn a_second_edit_restarts_the_debounce(cx: &mut TestAppContext) {
        let (path, document, _autosave) = harness("restart", PersistenceMode::ReadWrite, cx);

        add_node(&document, cx);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();

        add_node(&document, cx);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert!(!path.exists(), "the second edit pushed the deadline out");

        cx.executor().advance_clock(DEBOUNCE);
        cx.run_until_parked();
        let written = CanvasDocument::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written.active_page().unwrap().nodes.len(), 2);
    }

    #[gpui_kit::test]
    fn selection_changes_never_schedule_a_write(cx: &mut TestAppContext) {
        let (path, document, _autosave) = harness("selection", PersistenceMode::ReadWrite, cx);
        add_node(&document, cx);
        cx.run_until_parked();
        cx.executor().advance_clock(DEBOUNCE * 2);
        cx.run_until_parked();
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();

        document.update(cx, |document, cx| {
            document.select_all();
            cx.notify();
        });
        cx.run_until_parked();
        cx.executor().advance_clock(DEBOUNCE * 2);
        cx.run_until_parked();

        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), first);
    }

    #[gpui_kit::test]
    fn read_only_never_writes(cx: &mut TestAppContext) {
        let (path, document, _autosave) = harness("readonly", PersistenceMode::ReadOnly, cx);

        add_node(&document, cx);
        cx.run_until_parked();
        cx.executor().advance_clock(DEBOUNCE * 2);
        cx.run_until_parked();

        assert!(!path.exists(), "read-only mode must not touch disk");
    }
}
