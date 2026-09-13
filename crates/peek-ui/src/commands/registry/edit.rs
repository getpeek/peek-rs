use super::super::{
    CANVAS_NOT_TYPING, Command, Group, actions, always, has_selected_nodes, has_selection,
};

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "Edit::SelectAll",
        title: "Select all nodes",
        label: None,
        group: Group::Edit,
        keywords: "",
        default_keys: &["meta-a"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::edit::SelectAll),
        available: always,
    },
    Command {
        id: "Edit::DeleteSelection",
        title: "Delete selection",
        label: None,
        group: Group::Edit,
        keywords: "remove backspace node edge",
        default_keys: &["backspace"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::edit::DeleteSelection),
        available: has_selection,
    },
    Command {
        id: "Edit::Copy",
        title: "Copy selection",
        label: None,
        group: Group::Edit,
        keywords: "clipboard cells rows tsv nodes",
        default_keys: &["meta-c"],
        // One key, two meanings, chosen by focus — the reference's shape. The canvas copies the
        // selected nodes; the result table binds the same action on `RESULT_NODE`, which is
        // deeper on the dispatch path, so a focused table copies its cells as TSV instead. The
        // table publishes `DataTable`, not `Input`, so this predicate does reach it.
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::edit::copy::Copy),
        // Not `has_selection`: an edge has nothing to put on a clipboard, and the table's TSV
        // copy needs its node selected too.
        available: has_selected_nodes,
    },
    Command {
        id: "Edit::Cut",
        title: "Cut selection",
        label: None,
        group: Group::Edit,
        keywords: "clipboard move nodes",
        default_keys: &["meta-x"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::edit::Cut),
        available: has_selected_nodes,
    },
    Command {
        id: "Edit::Paste",
        title: "Paste",
        label: None,
        group: Group::Edit,
        keywords: "clipboard nodes duplicate",
        default_keys: &["meta-v"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::edit::Paste),
        available: always,
    },
];
