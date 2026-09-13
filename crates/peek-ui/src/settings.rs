//! `settings.json` as a gpui global, so a command can flip a preference and every view that
//! reads it repaints.
//!
//! The alternative — threading `Rc<PeekConfig>` down from `WorkspaceView` — makes each toggle
//! a change to every view between the command and the widget it controls. A global is what
//! `database.rs` already does for the same reason, and `cx.observe_global` gives the repaint
//! for free.

use gpui_kit::{App, BorrowAppContext, Global};
use peek_config::{ConfigError, PeekConfig, PersistenceMode};

#[derive(Debug)]
pub(crate) struct Settings {
    config: PeekConfig,
    persistence: PersistenceMode,
}

impl Global for Settings {}

impl Settings {
    pub(crate) fn init(config: PeekConfig, persistence: PersistenceMode, cx: &mut App) {
        cx.set_global(Self {
            config,
            persistence,
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
