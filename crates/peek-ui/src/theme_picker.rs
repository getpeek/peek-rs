//! The theme picker: a searchable list over the built-in themes with live preview while
//! browsing (arrow keys or hover), commit on Enter or click, and revert on Escape or
//! click-away. Built on the same palette component as the command palette so keyboard and
//! dismissal behaviour stay identical.

use gpui_kit::component::IndexPath;
use gpui_kit::component::WindowExt;
use gpui_kit::component::command::{Command as Palette, CommandItem, CommandState};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Window};
use peek_config::{PeekConfig, PersistenceMode, ThemeId};
use peek_theme::{ThemeService, builtin};

/// Opens the picker over the window. `state` is the retained palette state the owner keeps.
pub(crate) fn open(
    state: &Entity<CommandState>,
    persistence: PersistenceMode,
    window: &mut Window,
    cx: &mut App,
) {
    let current = ThemeService::committed(cx);
    let selected = ThemeId::ALL
        .iter()
        .position(|id| *id == current)
        .unwrap_or(0);
    state.update(cx, |state, cx| {
        state.set_selected_index(Some(IndexPath::new(selected)), window, cx);
    });

    let dialog_state = state.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let state = dialog_state.clone();
        crate::workspace::bare(dialog)
            .overlay_closable(true)
            .on_close(|_, _, cx| ThemeService::cancel_preview(cx))
            .content(move |content, _, _| {
                let items = builtin::all().map(|spec| {
                    CommandItem::new()
                        .label(spec.name)
                        .keywords([spec.tagline, if spec.is_light { "light" } else { "dark" }])
                });
                content.child(
                    Palette::new(&state)
                        .placeholder("Change theme")
                        .items(items)
                        .on_select(|path, _, cx| {
                            if let Some(id) = ThemeId::ALL.get(path.row) {
                                ThemeService::preview(*id, cx);
                            }
                        })
                        .on_confirm(move |path, window, cx| {
                            window.close_dialog(cx);
                            if let Some(id) = ThemeId::ALL.get(path.row) {
                                commit(*id, persistence, cx);
                            }
                        })
                        .on_cancel(|window, cx| {
                            ThemeService::cancel_preview(cx);
                            window.close_dialog(cx);
                        }),
                )
            })
    });
    state.update(cx, |state, cx| state.focus(window, cx));
}

fn commit(id: ThemeId, persistence: PersistenceMode, cx: &mut App) {
    if !ThemeService::commit(id, cx) {
        return;
    }
    let mut config = PeekConfig::get_or_default();
    config.theme = id;
    match config.save_to_disk(persistence) {
        Ok(()) => log::info!("peek: theme {} saved", id.as_str()),
        Err(error) => log::info!("peek: theme {} not saved: {error}", id.as_str()),
    }
}
