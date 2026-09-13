//! Adding, renaming and removing a workspace, ported from
//! `~/labs/peek/src/Connection/WorkspaceForm.tsx`.
//!
//! A workspace is a name and nothing else, so the form is one field — and the mascot preview,
//! which is the only thing that makes two workspaces starting with the same letter visibly the
//! same at a glance before you commit to the name.

use gpui_kit::AppContext;
use gpui_kit::base::input::InputState;
use gpui_kit::{App, Context, Entity, SharedString, Window};
use peek_config::PersistenceMode;
use peek_document::DocumentStore;

use super::PickerView;
use crate::settings::Settings;

pub(super) struct WorkspaceForm {
    /// `None` for a new workspace; otherwise the name it had when the form opened.
    pub(super) editing: Option<SharedString>,
    pub(super) name: Entity<InputState>,
    /// How many connections a removal would take with it, so the confirmation can say.
    pub(super) connections: usize,
    pub(super) error: Option<SharedString>,
    pub(super) confirming_remove: bool,
}

impl WorkspaceForm {
    pub(super) fn new(
        editing: Option<&str>,
        window: &mut Window,
        cx: &mut Context<PickerView>,
    ) -> Self {
        let connections = editing
            .and_then(|name| Settings::get(cx).workspace(name))
            .map_or(0, |workspace| workspace.connections.len());
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. Orchard")
                .default_value(editing.unwrap_or_default().to_string())
        });
        Self {
            editing: editing.map(|name| SharedString::from(name.to_string())),
            name,
            connections,
            error: None,
            confirming_remove: false,
        }
    }

    pub(super) fn value(&self, cx: &App) -> String {
        self.name.read(cx).value().trim().to_string()
    }

    pub(super) fn can_save(&self, cx: &App) -> bool {
        !self.value(cx).is_empty()
    }

    /// The letter tile, previewed at the size the form draws it.
    pub(super) fn mascot(&self, cx: &App) -> String {
        self.value(cx)
            .chars()
            .next()
            .map_or_else(|| "·".to_string(), |first| first.to_uppercase().to_string())
    }
}

/// Writes the form to `settings.json`, moving the workspace's directory first when the name
/// changed — same ordering rule as a connection rename, and for the same reason.
///
/// Returns the old and new names on a rename, so the caller can tell whether the workspace that
/// moved is the one on screen. Returning only the new name would make every rename look like the
/// open one had moved.
pub(super) fn commit(
    form: &WorkspaceForm,
    mode: PersistenceMode,
    cx: &mut App,
) -> Result<Option<(String, String)>, SharedString> {
    let name = form.value(cx);

    let Some(previous) = form.editing.clone() else {
        super::connection_form::write(cx, |config| config.add_workspace(&name))?;
        return Ok(None);
    };

    if previous.eq_ignore_ascii_case(&name) && previous == name {
        return Ok(None);
    }
    move_directory(&previous, &name, mode)?;
    let from = previous.to_string();
    super::connection_form::write(cx, |config| config.rename_workspace(&from, &name))?;
    Ok(Some((from, name)))
}

fn move_directory(from: &str, to: &str, mode: PersistenceMode) -> Result<(), SharedString> {
    if !mode.can_write() {
        return Ok(());
    }
    let store = DocumentStore::new(mode).map_err(|error| SharedString::from(error.to_string()))?;
    store
        .rename_workspace(from, to)
        .map_err(|error| SharedString::from(format!("could not move the workspace: {error}")))
}

// ---------------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------------

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{FontWeight, TestSupportExt, div, px};
use peek_theme::ActivePeekTheme;

pub(super) fn body(form: &WorkspaceForm, cx: &mut Context<PickerView>) -> impl IntoElement {
    let (fg, border, mascot_bg) = {
        let theme = cx.peek_theme();
        (theme.fg, theme.node_border, theme.node_bg_2)
    };
    let adding = form.editing.is_none();
    let title = form.editing.clone().map_or_else(
        || SharedString::from("New workspace"),
        |name| SharedString::from(format!("{name} · workspace")),
    );
    let mascot = form.mascot(cx);

    div()
        .id("workspace-form")
        .test_support()
        .v_flex()
        .flex_1()
        .min_h_0()
        .text_color(fg)
        .text_size(px(12.5))
        .child(super::list::form_header(title, cx))
        .child(fields(form, (mascot, mascot_bg), cx))
        .child(
            div()
                .h_flex()
                .justify_between()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .py(px(10.0))
                .border_t_1()
                .border_color(border)
                .child(
                    div()
                        .h_flex()
                        .gap(px(6.0))
                        .children((!adding).then(|| remove_control(form, cx))),
                )
                .child(
                    div()
                        .h_flex()
                        .gap(px(6.0))
                        .child(
                            Button::new("workspace-cancel")
                                .ghost()
                                .xsmall()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.back_to_list(window, cx);
                                })),
                        )
                        .child(save_control(form, adding, cx)),
                ),
        )
}

/// The mascot preview beside the name, the hint, and any refusal.
fn fields(
    form: &WorkspaceForm,
    mascot: (String, gpui_kit::Hsla),
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let (letter, background) = mascot;
    let (fg_subtle, red, red_soft) = {
        let theme = cx.peek_theme();
        (theme.fg_subtle, theme.red, theme.red_soft)
    };

    div()
        .v_flex()
        .gap(px(12.0))
        .px(px(18.0))
        .py(px(14.0))
        .child(
            div()
                .h_flex()
                .gap(px(12.0))
                .items_center()
                .child(
                    div()
                        .size(px(44.0))
                        .flex_shrink_0()
                        .rounded(px(10.0))
                        .bg(background)
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family("Monaspace Krypton")
                        .text_size(px(18.0))
                        .font_weight(FontWeight::BOLD)
                        .child(letter),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&form.name).small()),
                ),
        )
        .child(div().text_size(px(11.0)).text_color(fg_subtle).child(
            "A workspace groups connections. Its canvases live in a folder of the same name.",
        ))
        .children(form.error.clone().map(move |message| {
            div()
                .p(px(8.0))
                .rounded(px(6.0))
                .bg(red_soft)
                .text_size(px(11.5))
                .text_color(red)
                .child(message)
        }))
}

fn save_control(
    form: &WorkspaceForm,
    adding: bool,
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let writable = Settings::can_write(cx);
    Button::new("workspace-save")
        .primary()
        .xsmall()
        .label(if adding { "Create workspace" } else { "Save" })
        .disabled(!writable || !form.can_save(cx))
        .when(!writable, |button| {
            button.tooltip("Read-only: relaunch with --write to change settings.json")
        })
        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
}

/// Removing a workspace takes its connections with it, so the confirmation counts them. Their
/// canvases stay on disk, which is what makes this recoverable by hand.
fn remove_control(form: &WorkspaceForm, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    if !form.confirming_remove {
        return div().child(
            Button::new("workspace-remove")
                .ghost()
                .xsmall()
                .label("Remove")
                .text_color(theme.red)
                .on_click(cx.listener(|this, _, _, cx| this.arm_remove(cx))),
        );
    }

    let name = form.editing.clone().unwrap_or_default();
    let detail = match form.connections {
        0 => format!("Remove \u{201c}{name}\u{201d}?"),
        1 => format!("Remove \u{201c}{name}\u{201d} and its 1 connection?"),
        count => format!("Remove \u{201c}{name}\u{201d} and its {count} connections?"),
    };
    div()
        .h_flex()
        .gap(px(6.0))
        .items_center()
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.fg_subtle)
                .child(SharedString::from(detail)),
        )
        .child(
            Button::new("workspace-remove-confirm")
                .danger()
                .xsmall()
                .label("Remove")
                .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))),
        )
}
