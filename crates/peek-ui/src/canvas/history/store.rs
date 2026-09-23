//! Automatic checkpoints, ported from `useHistoryCapture.ts` and the IO half of
//! `historyStore.ts`.
//!
//! Checkpoints are coarse versions, not undo steps (those coalesce at 300 ms): one is taken 30
//! seconds after the last edit, but a long editing streak never goes more than three minutes
//! without one, and leaving a page captures it at once.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::{Context, Entity, Subscription, Task};
use peek_canvas::Document;
use peek_document::history::{
    Checkpoint, ConnectionHistory, HistoryEntry, HistoryFile, PageSnapshot,
};
use peek_document::{CheckpointId, PageId, StorageError};

pub(super) const DEBOUNCE: Duration = Duration::from_secs(30);
const MAX_WAIT: Duration = Duration::from_secs(180);

pub(crate) struct VersionHistory {
    document: Entity<Document>,
    history: ConnectionHistory,
    /// `None` keeps the history in memory only: the workspace a test builds has no file.
    file: Option<HistoryFile>,
    /// Captures wait for the log: diffing against an empty chain would write a full snapshot
    /// of every page, and the next load would find two chains' worth of first versions.
    loaded: bool,
    load: Option<Task<()>>,
    /// An edit's timer fired before the log arrived; the capture it owed runs on arrival.
    owed: bool,
    seen_revision: u64,
    active_page: PageId,
    /// Dropping a task cancels it, so reassigning is restarting.
    debounce: Option<Task<()>>,
    max_wait: Option<Task<()>>,
    _observer: Subscription,
    _quit: Subscription,
}

impl std::fmt::Debug for VersionHistory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VersionHistory")
            .field("file", &self.file.as_ref().map(HistoryFile::path))
            .field("loaded", &self.loaded)
            .finish_non_exhaustive()
    }
}

impl VersionHistory {
    pub(crate) fn new(
        document: Entity<Document>,
        file: Option<HistoryFile>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (seen_revision, active_page) = {
            let document = document.read(cx);
            (document.revision(), document.active_page_id().clone())
        };
        let load = file.clone().map(|file| {
            cx.spawn(async move |this, cx| {
                let parsed = cx
                    .background_executor()
                    .spawn(async move { load(&file) })
                    .await;
                this.update(cx, |this, cx| this.adopt(parsed, cx)).ok();
            })
        });
        Self {
            _observer: cx.observe(&document, |this, _, cx| this.on_document_changed(cx)),
            _quit: cx.on_app_quit(|this, cx| {
                this.flush(cx);
                async {}
            }),
            loaded: load.is_none(),
            load,
            owed: false,
            document,
            history: ConnectionHistory::default(),
            file,
            seen_revision,
            active_page,
            debounce: None,
            max_wait: None,
        }
    }

    fn adopt(&mut self, parsed: ConnectionHistory, cx: &mut Context<Self>) {
        self.history = parsed;
        self.loaded = true;
        self.load = None;
        if std::mem::take(&mut self.owed) {
            self.flush(cx);
        }
        cx.notify();
    }

    /// Verified entries for `page`, oldest first.
    pub(crate) fn entries(&self, page: &PageId) -> &[HistoryEntry] {
        self.history.entries(page)
    }

    pub(crate) fn reconstruct(&self, page: &PageId, entry: &CheckpointId) -> Option<PageSnapshot> {
        self.history.reconstruct(page, entry)
    }

    fn on_document_changed(&mut self, cx: &mut Context<Self>) {
        let document = self.document.read(cx);
        let revision = document.revision();
        if revision == self.seen_revision {
            return;
        }
        self.seen_revision = revision;
        // The debounce window must not ride across a context switch: the page being left is
        // captured as it was left.
        if document.active_page_id() != &self.active_page {
            self.active_page = document.active_page_id().clone();
            self.flush(cx);
            return;
        }
        self.debounce = Some(Self::flush_after(DEBOUNCE, cx));
        if self.max_wait.is_none() {
            self.max_wait = Some(Self::flush_after(MAX_WAIT, cx));
        }
    }

    fn flush_after(delay: Duration, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, Self::flush).ok();
        })
    }

    /// Captures every page that differs from its chain's tail.
    pub(crate) fn flush(&mut self, cx: &mut Context<Self>) {
        self.debounce = None;
        self.max_wait = None;
        if !self.loaded {
            self.owed = true;
            return;
        }
        let pages: Vec<PageId> = self
            .document
            .read(cx)
            .pages()
            .map(|page| page.id.clone())
            .collect();
        for page in pages {
            self.capture(&page, None, cx);
        }
    }

    /// Records `page` now, unless it matches its last checkpoint. Returns the entry that ends
    /// the page's chain either way, which is what the timeline calls the present.
    pub(crate) fn capture(
        &mut self,
        page: &PageId,
        label: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<CheckpointId> {
        if !self.loaded {
            return None;
        }
        let snapshot = self.document.read(cx).page_snapshot(page)?;
        let checkpoint = Checkpoint {
            page: page.clone(),
            snapshot,
            taken_at: now_ms(),
            label,
        };
        if let Some(entry) = self.history.capture(checkpoint) {
            write(self.file.as_ref(), entry);
            cx.notify();
        }
        self.history
            .entries(page)
            .last()
            .map(|entry| entry.id.clone())
    }
}

fn load(file: &HistoryFile) -> ConnectionHistory {
    let contents = file.load().unwrap_or_else(|error| {
        log::error!("peek: could not read {}: {error}", file.path().display());
        String::new()
    });
    let (history, compacted) = ConnectionHistory::parse(&contents);
    if compacted {
        match file.rewrite(&history.serialize()) {
            Ok(()) | Err(StorageError::ReadOnly) => {}
            Err(error) => log::error!("peek: could not compact the version history: {error}"),
        }
    }
    history
}

/// One line, appended where it happened. A line is a few kilobytes and the log is only ever
/// appended to, so this stays on the main thread the way autosave's writes do, and ordering
/// is free.
fn write(file: Option<&HistoryFile>, entry: &HistoryEntry) {
    let Some(file) = file else {
        return;
    };
    let line = match serde_json::to_string(entry) {
        Ok(line) => line,
        Err(error) => {
            log::error!("peek: could not encode a checkpoint: {error}");
            return;
        }
    };
    match file.append(&line) {
        Ok(()) => {}
        Err(StorageError::ReadOnly) => {
            log::debug!("peek: read-only mode, checkpoint kept in memory");
        }
        Err(error) => log::error!("peek: could not record a checkpoint: {error}"),
    }
}

/// `Date.now()`, the unit `takenAt` is recorded in.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, TestAppContext};
    use peek_canvas::{Point, Rect, Size};
    use peek_config::PersistenceMode;
    use peek_document::{CanvasDocument, DocumentStore, NodeType};

    fn harness(
        tag: &str,
        cx: &mut TestAppContext,
    ) -> (std::path::PathBuf, Entity<Document>, Entity<VersionHistory>) {
        let base = std::env::temp_dir().join(format!("peek-rs-history-{tag}"));
        let _ = std::fs::remove_dir_all(&base);
        let file = DocumentStore::with_base(base, PersistenceMode::ReadWrite)
            .open_history("prod", "maindb");
        let path = file.path().to_path_buf();
        let (document, history) = cx.update(|cx| {
            let document = cx.new(|_| Document::load(CanvasDocument::empty()));
            let history = cx.new(|cx| VersionHistory::new(document.clone(), Some(file), cx));
            (document, history)
        });
        cx.run_until_parked();
        (path, document, history)
    }

    fn add_node(document: &Entity<Document>, cx: &mut TestAppContext) {
        document.update(cx, |document, cx| {
            document.create_node(
                NodeType::Text,
                Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 100.0)),
            );
            cx.notify();
        });
        cx.run_until_parked();
    }

    fn lines(path: &std::path::Path) -> usize {
        std::fs::read_to_string(path).map_or(0, |log| log.lines().count())
    }

    #[gpui_kit::test]
    fn records_thirty_seconds_after_the_last_edit(cx: &mut TestAppContext) {
        let (path, document, _history) = harness("debounce", cx);

        add_node(&document, cx);
        cx.executor().advance_clock(Duration::from_secs(20));
        cx.run_until_parked();
        add_node(&document, cx);
        cx.executor().advance_clock(Duration::from_secs(20));
        cx.run_until_parked();
        assert_eq!(lines(&path), 0, "the second edit restarted the debounce");

        cx.executor().advance_clock(DEBOUNCE);
        cx.run_until_parked();
        assert_eq!(lines(&path), 1);
    }

    #[gpui_kit::test]
    fn a_long_streak_is_recorded_after_three_minutes(cx: &mut TestAppContext) {
        let (path, document, _history) = harness("max-wait", cx);
        for _ in 0..8 {
            add_node(&document, cx);
            cx.executor().advance_clock(Duration::from_secs(25));
            cx.run_until_parked();
        }
        assert_eq!(lines(&path), 1);
    }

    #[gpui_kit::test]
    fn leaving_a_page_records_it_at_once(cx: &mut TestAppContext) {
        let (path, document, _history) = harness("switch", cx);
        add_node(&document, cx);
        document.update(cx, |document, cx| {
            let page = document.add_page(None, None);
            document.switch_page(&page);
            cx.notify();
        });
        cx.run_until_parked();
        // The page left behind, and the new one: every page differing from its chain is recorded.
        assert_eq!(lines(&path), 2);
    }

    #[gpui_kit::test]
    fn a_reloaded_log_continues_its_chain(cx: &mut TestAppContext) {
        let (path, document, history) = harness("reload", cx);
        add_node(&document, cx);
        history.update(cx, VersionHistory::flush);
        let page = cx.update(|cx| document.read(cx).active_page_id().clone());

        let file = DocumentStore::with_base(
            path.parent().unwrap().parent().unwrap().to_path_buf(),
            PersistenceMode::ReadWrite,
        )
        .open_history("prod", "maindb");
        let reloaded =
            cx.update(|cx| cx.new(|cx| VersionHistory::new(document.clone(), Some(file), cx)));
        cx.run_until_parked();
        add_node(&document, cx);
        reloaded.update(cx, VersionHistory::flush);

        cx.update(|cx| {
            let entries = reloaded.read(cx).entries(&page);
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[1].parent_id.as_ref(), Some(&entries[0].id));
            assert_eq!(entries[1].seq, 2);
        });
    }
}
