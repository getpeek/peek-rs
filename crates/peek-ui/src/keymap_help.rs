//! The keyboard-shortcut reference (`~/labs/peek/src/keymap-help/`).
//!
//! Every row is derived from the command registry and the effective keymap, so registering a
//! command or rebinding a key updates this dialog with no edit here. The reference keeps a
//! hand-written `KEYMAP_REFERENCE` array whose descriptions are copied from its
//! `docs/keymap.md`, and the two have already drifted apart: `Page::OpenPicker` is bound and
//! listed in the modal, and missing from the markdown it was copied from.
//!
//! The keymap is read rather than the window's live bindings, because a registry context may
//! be a predicate (`Canvas && !Input`), which `KeyContext::parse` aborts the process on, and
//! because a node-scoped binding resolves against no focus handle the dialog could hold — both
//! would turn real shortcuts into "Unbound". [`commands::keymap::resolved`] is the same
//! function that installed the bindings at startup, so it cannot disagree with them.

use std::collections::HashMap;

use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme, StyledExt, WindowExt};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, Keystroke, TestSupportExt, Window, div, px};

use crate::commands::{self, Group, keymap};
use crate::settings::Settings;

/// One registry group and the commands in it that take part in the keymap.
#[derive(Debug)]
struct Section {
    group: Group,
    rows: Vec<Row>,
}

#[derive(Debug)]
struct Row {
    title: &'static str,
    /// Every keystroke that reaches this command. Empty means a user override took the key
    /// this command had by default and gave it to something else.
    keys: Vec<String>,
}

pub(crate) fn open(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, _| {
        dialog
            .w(px(680.0))
            .title("Keyboard shortcuts")
            .content(|content, window, cx| {
                // Derived per frame rather than captured once: the dialog cannot go stale, and
                // there is no snapshot to keep in sync with the registry.
                let (left, right) = split(sections(&Settings::get(cx).keymap));
                content.child(
                    div()
                        .id("keymap-help")
                        .test_support()
                        .h_flex()
                        .items_start()
                        .gap_8()
                        // Enough of the window that the list is worth scrolling, and never so
                        // much that the dialog runs off the bottom of a minimum-size window.
                        .max_h(window.viewport_size().height * 0.6)
                        .overflow_y_scroll()
                        .child(column(&left, cx))
                        .child(column(&right, cx)),
                )
            })
    });
}

/// The registry, grouped, keeping only the commands a key can reach.
fn sections(overrides: &HashMap<String, String>) -> Vec<Section> {
    let resolved = keymap::resolved(overrides);
    let mut bound: HashMap<&str, Vec<String>> = HashMap::new();
    for (key, command) in &resolved {
        bound.entry(command.id).or_default().push(key.clone());
    }

    let mut sections: Vec<Section> = Vec::new();
    for command in commands::all() {
        let keys = bound.remove(command.id).unwrap_or_default();
        // A command with no key and no default is palette-only; listing every one of them
        // would bury the shortcuts under rows that read "Unbound".
        if keys.is_empty() && command.default_keys.is_empty() {
            continue;
        }
        let row = Row {
            title: command.title,
            keys,
        };
        match sections
            .iter_mut()
            .find(|section| section.group == command.group)
        {
            Some(section) => section.rows.push(row),
            None => sections.push(Section {
                group: command.group,
                rows: vec![row],
            }),
        }
    }
    sections
}

/// Splits the groups over two columns, as the reference's `column-count: 2` does. A heading
/// costs about two rows of height, so a group of one is not free.
fn split(sections: Vec<Section>) -> (Vec<Section>, Vec<Section>) {
    fn weight(section: &Section) -> usize {
        section.rows.len() + 2
    }

    let total: usize = sections.iter().map(weight).sum();
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut filled = 0;
    for section in sections {
        if filled * 2 < total {
            filled += weight(&section);
            left.push(section);
        } else {
            right.push(section);
        }
    }
    (left, right)
}

fn column(sections: &[Section], cx: &App) -> Div {
    div()
        .v_flex()
        .flex_1()
        .gap_4()
        .children(sections.iter().map(|section| group(section, cx)))
}

fn group(section: &Section, cx: &App) -> Div {
    div()
        .v_flex()
        .gap_1()
        .child(
            div()
                .pb_1()
                .text_xs()
                .font_semibold()
                .text_color(cx.theme().muted_foreground)
                .child(section.group.title().to_uppercase()),
        )
        .children(section.rows.iter().map(|row| shortcut(row, cx)))
}

fn shortcut(row: &Row, cx: &App) -> Div {
    div()
        .h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .min_h(px(26.0))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(row.title),
        )
        .child(keys(&row.keys, cx))
}

fn keys(keys: &[String], cx: &App) -> AnyElement {
    if keys.is_empty() {
        return div()
            .text_xs()
            .italic()
            .text_color(cx.theme().muted_foreground)
            .child("Unbound")
            .into_any_element();
    }

    div()
        .h_flex()
        .gap_1()
        .flex_shrink_0()
        .children(
            keys.iter()
                .filter_map(|key| Keystroke::parse(key).ok().map(Kbd::new)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_of(sections: &[Section], title: &str) -> (Group, Vec<String>) {
        sections
            .iter()
            .find_map(|section| {
                section
                    .rows
                    .iter()
                    .find(|row| row.title == title)
                    .map(|row| (section.group, row.keys.clone()))
            })
            .unwrap_or_else(|| panic!("{title} is listed"))
    }

    #[test]
    fn a_bound_command_is_listed_under_its_group_with_its_key() {
        let sections = sections(&HashMap::new());
        assert_eq!(
            row_of(&sections, "Show keymap"),
            (Group::Help, vec!["cmd-/".to_string()])
        );
    }

    /// The palette lists every command; a keyboard reference that did the same would be mostly
    /// rows with no key in them.
    #[test]
    fn a_command_no_key_reaches_is_left_out() {
        let sections = sections(&HashMap::new());
        assert!(
            !sections
                .iter()
                .flat_map(|section| &section.rows)
                .any(|row| row.title == "Change theme"),
            "Theme::Open has no default key and belongs in the palette only"
        );
    }

    /// Taking a command's default key away is exactly the case the reference shows "Unbound"
    /// for, and the only way a listed command ends up with no keys.
    #[test]
    fn a_command_whose_key_was_taken_still_shows_as_unbound() {
        let overrides = HashMap::from([("meta-/".to_string(), "Zoom::In".to_string())]);
        let sections = sections(&overrides);
        assert_eq!(row_of(&sections, "Show keymap"), (Group::Help, Vec::new()));
        assert!(
            row_of(&sections, "Zoom in")
                .1
                .contains(&"cmd-/".to_string()),
            "and the command that took it lists both keys"
        );
    }

    #[test]
    fn the_two_columns_are_within_one_group_of_balanced() {
        let (left, right) = split(sections(&HashMap::new()));
        assert!(!left.is_empty() && !right.is_empty());
        let rows = |sections: &[Section]| -> usize {
            sections.iter().map(|section| section.rows.len() + 2).sum()
        };
        let (left, right) = (rows(&left), rows(&right));
        assert!(
            left.abs_diff(right) * 3 < left + right,
            "columns differ by more than a third: {left} against {right}"
        );
    }
}
