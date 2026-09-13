//! Exporting a selected result node's rows to a file the user picks a directory for.
//!
//! Both handlers live on `CanvasView` (`canvas/dispatch/execution.rs`) rather than on the
//! result node: the palette dispatches through the canvas focus handle, which is an ancestor
//! of node elements, so a node-only handler would never be reached.

use super::super::{CANVAS_NOT_TYPING, Command, Group, actions, has_selected_results};

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "Export::Csv",
        title: "Export selected data (CSV)",
        label: None,
        group: Group::Export,
        keywords: "save download spreadsheet result rows",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::export::Csv),
        available: has_selected_results,
    },
    Command {
        id: "Export::Json",
        title: "Export selected data (JSON)",
        label: None,
        group: Group::Export,
        keywords: "save download result rows",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::export::Json),
        available: has_selected_results,
    },
];
