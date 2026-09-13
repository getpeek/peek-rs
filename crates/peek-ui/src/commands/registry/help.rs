use super::super::{Command, Group, WORKSPACE, actions, always};

pub(super) static ENTRIES: &[Command] = &[Command {
    id: "Help::Keymap",
    title: "Show keymap",
    label: None,
    group: Group::Help,
    keywords: "keybindings keyboard shortcuts hotkeys cheatsheet reference",
    default_keys: &["meta-/"],
    context: WORKSPACE,
    build: || Box::new(actions::help::Keymap),
    available: always,
}];
