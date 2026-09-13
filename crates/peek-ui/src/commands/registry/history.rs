use super::super::{CANVAS_NOT_TYPING, Command, Group, actions, can_redo, can_undo};

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "History::Undo",
        title: "Undo",
        label: None,
        group: Group::History,
        keywords: "revert back",
        default_keys: &["meta-z"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::history::Undo),
        available: can_undo,
    },
    Command {
        id: "History::Redo",
        title: "Redo",
        label: None,
        group: Group::History,
        keywords: "again forward",
        default_keys: &["meta-shift-z"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::history::Redo),
        available: can_redo,
    },
];
