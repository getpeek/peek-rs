//! Version history, ported from `~/labs/peek/src/canvas/history/`: coarse per-page checkpoints
//! in `~/peek/<workspace>/<connection>.history.jsonl`, one [`HistoryEntry`] per line. A full
//! snapshot every twentieth entry, deltas between. Separate from undo, which is in memory.

mod chain;
mod delta;
mod entry;
mod file;
pub mod format;

pub use chain::{Checkpoint, ConnectionHistory};
pub use delta::{apply, diff};
pub use entry::{ChangeSummary, EntryBody, HistoryEntry, PageDelta, PageSnapshot};
pub use file::HistoryFile;
