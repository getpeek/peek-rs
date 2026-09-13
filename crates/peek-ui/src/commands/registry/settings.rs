use super::super::{Command, Group, WORKSPACE, actions, always};
use peek_canvas::Scope;

/// Named for what pressing it does, not for the mode that is up — the reference's convention,
/// and the one `registry/view.rs`'s `interface_label` already follows.
fn page_display_label(scope: &Scope) -> &'static str {
    if scope.settings.pages_as_list {
        "Show pages as tabs"
    } else {
        "Show pages as list"
    }
}

fn palette_button_label(scope: &Scope) -> &'static str {
    if scope.settings.palette_button_hidden {
        "Show command palette button"
    } else {
        "Hide command palette button"
    }
}

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "Settings::ToggleCommandPaletteButton",
        // The stable name, for the keymap modal and tooltips; the palette shows `label`.
        title: "Show or hide the command palette button",
        label: Some(palette_button_label),
        group: Group::Settings,
        keywords: "titlebar search pill toggle setting command palette button",
        default_keys: &[],
        context: WORKSPACE,
        build: || Box::new(actions::settings::ToggleCommandPaletteButton),
        available: always,
    },
    Command {
        id: "Settings::TogglePageDisplay",
        // The stable name, for the keymap modal and tooltips; the palette shows `label`.
        title: "Show pages as tabs or list",
        label: Some(page_display_label),
        group: Group::Settings,
        keywords: "pages tabs list display show titlebar navigation toggle setting",
        default_keys: &[],
        context: WORKSPACE,
        build: || Box::new(actions::settings::TogglePageDisplay),
        available: always,
    },
];
