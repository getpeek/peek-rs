//! `settings.json` as a gpui global, so a command can flip a preference and every view that
//! reads it repaints.
//!
//! The alternative — threading `Rc<PeekConfig>` down from `WorkspaceView` — makes each toggle
//! a change to every view between the command and the widget it controls. A global is what
//! `database.rs` already does for the same reason, and `cx.observe_global` gives the repaint
//! for free.

use gpui_kit::{App, BorrowAppContext, Global};
use peek_config::{ConfigError, PeekConfig, PersistenceMode};

use crate::Launch;

#[derive(Debug)]
pub(crate) struct Settings {
    config: PeekConfig,
    persistence: PersistenceMode,
    performance: bool,
    fps: bool,
}

impl Global for Settings {}

impl Settings {
    pub(crate) fn init(config: PeekConfig, launch: &Launch, cx: &mut App) {
        cx.set_global(Self {
            config,
            persistence: launch.persistence,
            performance: launch.performance,
            fps: launch.fps,
        });
    }

    pub(crate) fn get(cx: &App) -> &PeekConfig {
        &cx.global::<Self>().config
    }

    /// The mode this run was launched in, for the paths that gate on it themselves —
    /// `load_document` and the autosave that must not exist under `ReadOnly`.
    pub(crate) fn persistence(cx: &App) -> PersistenceMode {
        cx.global::<Self>().persistence
    }

    /// Whether `--performance` was given: the canvas drops the body of a node the camera has
    /// zoomed too far out to read. Off by default, because the detail it trades away is real —
    /// a page of shells is harder to recognise than a page of nodes.
    pub(crate) fn performance(cx: &App) -> bool {
        cx.global::<Self>().performance
    }

    /// Turns that trade on for a test that is about it. `--performance` is a launch option, so
    /// this is the only way to move it once `init` has run.
    #[cfg(test)]
    pub(crate) fn set_performance(enabled: bool, cx: &mut App) {
        cx.update_global::<Self, _>(|settings, _| settings.performance = enabled);
    }

    /// Whether `--fps` was given: the zoom cluster carries a frame-rate readout, and the canvas
    /// samples the wall clock once a frame to feed it. Read once, in `CanvasView::new`, because
    /// the flag cannot move for the life of the process.
    pub(crate) fn fps(cx: &App) -> bool {
        cx.global::<Self>().fps
    }

    /// Whether a write would reach disk at all, so a form can disable Save and say why rather
    /// than accepting an edit it is about to drop.
    pub(crate) fn can_write(cx: &App) -> bool {
        cx.global::<Self>().persistence.can_write()
    }

    /// Applies `change` and writes `settings.json`.
    ///
    /// The in-memory change stands either way — a toggle still takes effect for this run under
    /// [`PersistenceMode::ReadOnly`] — but the write result is returned rather than logged,
    /// because a form that just took a connection string from someone has to say why it did
    /// not persist. Callers with nothing to tell the user can `let _ =` it.
    pub(crate) fn update(
        cx: &mut App,
        change: impl FnOnce(&mut PeekConfig),
    ) -> Result<(), ConfigError> {
        cx.update_global::<Self, _>(|settings, _| {
            change(&mut settings.config);
            settings.config.save_to_disk(settings.persistence)
        })
    }
}
