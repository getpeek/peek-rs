//! Theme switching with a live preview: the picker previews while the user browses, commits on
//! Enter and cancels on Escape. Persistence is the caller's concern (it is gated on
//! `PersistenceMode`), so this module does no IO.

use gpui_kit::component::theme::Theme;
use gpui_kit::{App, Global};
use peek_config::ThemeId;

use crate::builtin;
use crate::component_map::to_component_config;
use crate::resolved::PeekTheme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeService {
    committed: ThemeId,
    preview: Option<ThemeId>,
}

impl Global for ThemeService {}

impl ThemeService {
    /// Installs the service and applies `initial` to both theme globals.
    pub fn init(initial: ThemeId, cx: &mut App) {
        cx.set_global(Self {
            committed: initial,
            preview: None,
        });
        apply(initial, cx);
    }

    /// The theme on screen: the preview while one is active, else the committed one.
    #[must_use]
    pub fn effective(cx: &App) -> ThemeId {
        let service = cx.global::<Self>();
        service.preview.unwrap_or(service.committed)
    }

    #[must_use]
    pub fn committed(cx: &App) -> ThemeId {
        cx.global::<Self>().committed
    }

    /// Shows `id` without committing to it.
    pub fn preview(id: ThemeId, cx: &mut App) {
        if Self::effective(cx) == id {
            cx.global_mut::<Self>().preview = Some(id);
            return;
        }
        cx.global_mut::<Self>().preview = Some(id);
        apply(id, cx);
    }

    /// Makes `id` the theme and clears any preview. Returns whether it changed.
    pub fn commit(id: ThemeId, cx: &mut App) -> bool {
        let previous = Self::effective(cx);
        let service = cx.global_mut::<Self>();
        let changed = service.committed != id;
        service.committed = id;
        service.preview = None;
        if previous != id {
            apply(id, cx);
        }
        changed
    }

    /// Drops the preview and shows the committed theme again.
    pub fn cancel_preview(cx: &mut App) {
        let service = cx.global_mut::<Self>();
        let Some(preview) = service.preview.take() else {
            return;
        };
        let committed = service.committed;
        if preview != committed {
            apply(committed, cx);
        }
    }
}

fn apply(id: ThemeId, cx: &mut App) {
    let spec = builtin::spec(id);
    let config = to_component_config(spec);
    let mode = config.mode;
    Theme::global_mut(cx).apply_config(&config);
    // `change` re-applies the slot `apply_config` just filled, resolves the mono font and
    // pushes the projection to the Base layer (scrollbars, resize handles).
    Theme::change(mode, None, cx);
    cx.set_global(PeekTheme::from_spec(spec));
    cx.refresh_windows();
    log::debug!("peek: theme {} applied", spec.name);
}
